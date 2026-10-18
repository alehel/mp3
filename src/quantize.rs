//! Non-uniform quantization, rate loop and distortion-control loop.
//!
//! **Milestone:** M3 (inner loop), M4 (outer loop, scalefactors). See docs/PLAN.md.
//!
//! **Allowed sources:** ISO/IEC 11172-3 Annex C.1.5.4 (informative encoder description).
//!
//! **Must be tested by:** Round-trip: quantize -> dequantize per §2.4.3.4.7 must reproduce magnitudes within quantizer error.
//!
//! Clean-room reminder: derive from the sources above only. Cite clauses in
//! comments and log every source in PROVENANCE.md (CLEANROOM.md).
//!
//! Status: M3. No scalefactors yet (all zero, `scalefac_compress` = 0), so the
//! only control is `global_gain`, chosen by the rate loop.

use crate::huffman::{self, HuffmanPlan, MAX_BIG_VALUE};
pub use crate::mdct::GRANULE_LINES;

/// The decoder requantizes with `2^((global_gain − 210) / 4)` (published
/// 11172-3 §2.4.3.4 "Formula for requantization and all scaling"; the
/// Committee Draft prints a different system constant, 64, tied to its own
/// filterbank scaling). Our hybrid filterbank is scaled for unit gain
/// through the standard's synthesis, so with 210 a decoder reproduces the
/// input level; `tests/decode.rs` checks this against ffmpeg and minimp3.
pub const GLOBAL_GAIN_OFFSET: i32 = 210;

/// Rounding offset in `ix = nint(v^0.75 − 0.0946)` (Annex C.1.5.4 §4.1).
///
/// original derivation: the decoder reconstructs `|ix|^(4/3)`, so the
/// MSE-optimal decision threshold between levels 0 and 1, in the companded
/// domain, is the 3/4 power of the midpoint between 0 and 1:
/// `0.5^0.75 = 0.5946`, i.e. round-half-up shifted by 0.0946. For larger
/// levels the exact midpoint tends to `ix + 0.5`, so this offset biases them
/// slightly downwards, which we accept as Annex C does.
const ROUNDING_OFFSET: f32 = 0.0946;

/// Long-block scalefactor band boundaries for one sample rate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalefactorBands {
    /// Start line of bands 0..=21, then 576.
    bounds: [usize; 23],
}

/// Long-block band widths, bands 0..=20. MPEG-1: 11172-3 Table 3-B.8a–c
/// (band 21 is the remainder up to line 575). LSF: 13818-3 Table B.2
/// (its band 21 is listed and also ends at line 575).
const WIDTHS_44100: [usize; 21] = [
    4, 4, 4, 4, 4, 4, 6, 6, 8, 8, 10, 12, 16, 20, 24, 28, 34, 42, 50, 54, 76,
];
const WIDTHS_48000: [usize; 21] = [
    4, 4, 4, 4, 4, 4, 6, 6, 6, 8, 10, 12, 16, 18, 22, 28, 34, 40, 46, 54, 54,
];
const WIDTHS_32000: [usize; 21] = [
    4, 4, 4, 4, 4, 4, 6, 6, 8, 10, 12, 16, 20, 24, 30, 38, 46, 56, 68, 84, 102,
];
/// 22.05 and 16 kHz share one table in 13818-3 Table B.2.
const WIDTHS_22050_16000: [usize; 21] = [
    6, 6, 6, 6, 6, 6, 8, 10, 12, 14, 16, 20, 24, 28, 32, 38, 46, 52, 60, 68, 58,
];
const WIDTHS_24000: [usize; 21] = [
    6, 6, 6, 6, 6, 6, 8, 10, 12, 14, 16, 18, 22, 26, 32, 38, 46, 54, 62, 70, 76,
];

impl ScalefactorBands {
    /// Long-block bands for a sample rate, or `None` where no table is
    /// available (MPEG-2.5 rates, which no ISO standard defines).
    pub fn long(sample_rate: u32) -> Option<Self> {
        let widths = match sample_rate {
            44_100 => &WIDTHS_44100,
            48_000 => &WIDTHS_48000,
            32_000 => &WIDTHS_32000,
            22_050 | 16_000 => &WIDTHS_22050_16000,
            24_000 => &WIDTHS_24000,
            _ => return None,
        };
        let mut bounds = [GRANULE_LINES; 23];
        let mut pos = 0;
        for (b, w) in bounds.iter_mut().zip(widths) {
            *b = pos;
            pos += w;
        }
        bounds[21] = pos;
        Some(Self { bounds })
    }

    /// Band boundaries used for Huffman region splits: start of each band
    /// 0..=21 followed by 576.
    pub fn region_bounds(&self) -> &[usize] {
        &self.bounds
    }
}

/// Quantize a granule at a given `global_gain` (Annex C.1.5.4 §4.1):
/// `ix = sign(xr) · nint((|xr| / 2^((global_gain − 210)/4))^0.75 − 0.0946)`.
/// Returns the largest magnitude.
pub fn quantize(xr: &[f32; GRANULE_LINES], global_gain: u8, ix: &mut [i32; GRANULE_LINES]) -> u32 {
    let inv_step = 2f32.powf(-(i32::from(global_gain) - GLOBAL_GAIN_OFFSET) as f32 / 4.0);
    let mut max = 0;
    for (q, &x) in ix.iter_mut().zip(xr) {
        let mag = ((x.abs() * inv_step).powf(0.75) - ROUNDING_OFFSET + 0.5).floor();
        // Saturate rather than overflow; the rate loop rejects values above
        // MAX_BIG_VALUE anyway.
        let m = mag.min(1e6) as u32;
        max = max.max(m);
        *q = if x < 0.0 { -(m as i32) } else { m as i32 };
    }
    max
}

/// A quantized, Huffman-planned granule.
#[derive(Debug, Clone)]
pub struct QuantizedGranule {
    pub global_gain: u8,
    pub ix: [i32; GRANULE_LINES],
    pub plan: HuffmanPlan,
}

/// Inner (rate control) loop, Annex C.1.5.4 §4: find the smallest
/// `global_gain` (finest quantizer) whose Huffman coding fits `max_bits`.
///
/// original: binary search over 0..=255 instead of the Annex's step-by-one
/// increase. Bit count is close to monotonic in `global_gain`, so this finds
/// the same answer or one a step coarser in ~8 trials. The lower bound is the
/// smallest gain keeping every magnitude ≤ 8191 (§4.2).
pub fn rate_loop(
    xr: &[f32; GRANULE_LINES],
    bands: &ScalefactorBands,
    max_bits: u32,
) -> QuantizedGranule {
    let mut ix = [0i32; GRANULE_LINES];
    let try_gain = |gg: u8, ix: &mut [i32; GRANULE_LINES]| -> Option<HuffmanPlan> {
        (quantize(xr, gg, ix) <= MAX_BIG_VALUE).then(|| huffman::plan(ix, bands))
    };

    let (mut lo, mut hi) = (0u8, 255u8);
    // Raise lo until magnitudes fit the tables.
    while try_gain(lo, &mut ix).is_none() && lo < 255 {
        lo = lo.saturating_add(8);
    }
    while lo > 0 && try_gain(lo - 1, &mut ix).is_some() {
        lo -= 1;
    }
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        match try_gain(mid, &mut ix) {
            Some(p) if p.bits <= max_bits => hi = mid,
            _ => lo = mid + 1,
        }
    }
    let plan = try_gain(hi, &mut ix).expect("gain >= lower bound always fits the tables");
    QuantizedGranule {
        global_gain: hi,
        ix,
        plan,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_tables_cover_the_spectrum() {
        for (rate, last_end) in [
            (44_100, 418),
            (48_000, 384),
            (32_000, 550),
            (22_050, 522),
            (24_000, 540),
            (16_000, 522),
        ] {
            let b = ScalefactorBands::long(rate).unwrap();
            let r = b.region_bounds();
            assert_eq!(r[0], 0);
            assert_eq!(r[21], last_end, "{rate}: start of band 21");
            assert_eq!(r[22], 576);
            assert!(r.windows(2).all(|w| w[0] < w[1]), "{rate}");
        }
        assert!(ScalefactorBands::long(11_025).is_none());
    }

    #[test]
    fn quantizer_known_values() {
        // At global_gain 210 the step is 1: ix = nint(|xr|^0.75 - 0.0946).
        let cases = [
            (0.0f32, 0),
            (0.49, 0),  // 0.49^0.75 = 0.586 < 0.5946
            (0.51, 1),  // 0.51^0.75 = 0.604
            (-1.0, -1), // sign kept
            (8.0, 5),   // 8^0.75 = 4.757 -> nint(4.662) = 5
        ];
        let mut xr = [0.0f32; GRANULE_LINES];
        for (x, &(v, _)) in xr.iter_mut().zip(&cases) {
            *x = v;
        }
        let mut ix = [0; GRANULE_LINES];
        quantize(&xr, 210, &mut ix);
        for (i, &(v, expect)) in cases.iter().enumerate() {
            assert_eq!(ix[i], expect, "xr = {v}");
        }
        // Four gain steps double the step size: xr = 16 at gain 214
        // quantizes like 8 at gain 210.
        xr[0] = 16.0;
        quantize(&xr, 214, &mut ix);
        assert_eq!(ix[0], 5);
    }

    #[test]
    fn dequantize_within_quantizer_error() {
        // Requantize per §2.4.3.4 (no scalefactors): |ix|^(4/3) · 2^((gg-210)/4).
        let mut xr = [0.0f32; GRANULE_LINES];
        for (i, x) in xr.iter_mut().enumerate() {
            *x = ((i as f32 * 0.37).sin()) * (1.0 + i as f32);
        }
        for gg in [150u8, 190, 210, 230] {
            let mut ix = [0; GRANULE_LINES];
            quantize(&xr, gg, &mut ix);
            let step = 2f64.powf((f64::from(gg) - 210.0) / 4.0);
            for (&q, &x) in ix.iter().zip(&xr) {
                let rec =
                    f64::from(q.signum()) * f64::from(q.unsigned_abs()).powf(4.0 / 3.0) * step;
                // Error is at most half a level in the companded domain:
                // bound by the reconstruction spacing around |ix|.
                let a = f64::from(q.unsigned_abs());
                let spacing = ((a + 1.0).powf(4.0 / 3.0) - a.powf(4.0 / 3.0)) * step;
                assert!(
                    (rec - f64::from(x)).abs() <= spacing,
                    "gg {gg}: x {x} ix {q}"
                );
            }
        }
    }

    #[test]
    fn rate_loop_meets_budget() {
        let bands = ScalefactorBands::long(44_100).unwrap();
        let mut xr = [0.0f32; GRANULE_LINES];
        for (i, x) in xr.iter_mut().enumerate() {
            *x = 0.05 * ((i * 7919 % 1000) as f32 / 500.0 - 1.0) / (1.0 + i as f32 / 50.0);
        }
        for budget in [200u32, 800, 2000, 4000] {
            let q = rate_loop(&xr, &bands, budget);
            assert!(
                q.plan.bits <= budget,
                "budget {budget}: {} bits",
                q.plan.bits
            );
            // One step finer must not fit (or exceed the table range).
            if q.global_gain > 0 {
                let mut ix = [0; GRANULE_LINES];
                if quantize(&xr, q.global_gain - 1, &mut ix) <= MAX_BIG_VALUE {
                    let finer = huffman::plan(&ix, &bands);
                    assert!(
                        finer.bits > budget,
                        "budget {budget}: gain {} not minimal",
                        q.global_gain
                    );
                }
            }
        }
    }
}
