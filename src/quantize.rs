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
//! Status: M5. The outer (distortion control) loop with long-block
//! scalefactors, `preflag` and `scalefac_scale`, for MPEG-1. Its allowed
//! distortion comes from psychoacoustic model 2 (`crate::psy`, the default)
//! or the absolute threshold of hearing (Annex D Table 3-D.4, experimental);
//! see [`outer_loop`] and [`crate::NoiseShaping`]. LSF granules use the rate
//! loop alone (their scalefactor coding differs; M8).

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
pub fn rate_loop(
    xr: &[f32; GRANULE_LINES],
    bands: &ScalefactorBands,
    max_bits: u32,
) -> QuantizedGranule {
    rate_loop_from(xr, bands, max_bits, None)
}

/// Smallest `global_gain` keeping every magnitude ≤ 8191 (Annex C.1.5.4
/// §4.2), by binary search on the quantizer alone (no bit counting).
fn min_gain(xr: &[f32; GRANULE_LINES], ix: &mut [i32; GRANULE_LINES]) -> u8 {
    let (mut lo, mut hi) = (0u8, 255u8);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if quantize(xr, mid, ix) <= MAX_BIG_VALUE {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}

/// [`rate_loop`] starting the search at `hint` (e.g. the previous outer-loop
/// iteration's gain).
///
/// original: galloping search from the hint, then binary search inside the
/// bracket, instead of the Annex's step-by-one increase. Bit count is close
/// to monotonic in `global_gain`, so this finds the same answer (or one a
/// step coarser) in a handful of trials when the hint is close.
pub fn rate_loop_from(
    xr: &[f32; GRANULE_LINES],
    bands: &ScalefactorBands,
    max_bits: u32,
    hint: Option<u8>,
) -> QuantizedGranule {
    let mut ix = [0i32; GRANULE_LINES];
    let floor = min_gain(xr, &mut ix);
    let mut fits = |gg: u8| {
        quantize(xr, gg, &mut ix);
        huffman::plan(&ix, bands).bits <= max_bits
    };

    // Bracket the answer: `lo` known not to fit (or below the floor), `hi`
    // known to fit (255 is accepted as a last resort).
    let start = hint.unwrap_or(210).max(floor);
    let (mut lo, mut hi): (i32, i32);
    if fits(start) {
        hi = i32::from(start);
        let mut step = 1;
        loop {
            let probe = hi - step;
            if probe < i32::from(floor) {
                lo = i32::from(floor) - 1;
                break;
            }
            if fits(probe as u8) {
                hi = probe;
                step *= 2;
            } else {
                lo = probe;
                break;
            }
        }
    } else {
        lo = i32::from(start);
        let mut step = 1;
        loop {
            let probe = (lo + step).min(255);
            if probe == 255 || fits(probe as u8) {
                hi = probe;
                break;
            }
            lo = probe;
            step *= 2;
        }
    }
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if fits(mid as u8) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let global_gain = hi as u8;
    quantize(xr, global_gain, &mut ix);
    let plan = huffman::plan(&ix, bands);
    QuantizedGranule {
        global_gain,
        ix,
        plan,
    }
}

/// `scalefac_compress` → `(slen1, slen2)` for MPEG-1 (11172-3 §2.4.2.7, as
/// restated in 13818-3 §2.5.2). slen1 covers bands 0–10, slen2 bands 11–20.
pub const SLEN: [(u8, u8); 16] = [
    (0, 0),
    (0, 1),
    (0, 2),
    (0, 3),
    (3, 0),
    (1, 1),
    (1, 2),
    (1, 3),
    (2, 1),
    (2, 2),
    (2, 3),
    (3, 1),
    (3, 2),
    (3, 3),
    (4, 2),
    (4, 3),
];

/// Preemphasis table, 11172-3 Table 3-B.6 (long-block bands 0..=20).
pub const PRETAB: [u8; 21] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 3, 2,
];

/// Scalefactor bands with a transmitted scalefactor (long blocks). Band 21
/// has none; the decoder uses 0 there (13818-3 §2.5.2, `scalefac_l`).
pub const SCALEFAC_BANDS: usize = 21;

/// Cheapest `scalefac_compress` able to carry `sf`, with its part2 length
/// (`11·slen1 + 10·slen2` bits, 11172-3 §2.4.2.7 `part2_length`).
pub fn choose_scalefac_compress(sf: &[u8; SCALEFAC_BANDS]) -> Option<(u8, u32)> {
    let need = |range: std::ops::Range<usize>| {
        let max = sf[range].iter().copied().max().unwrap_or(0);
        (u8::BITS - max.leading_zeros()) as u8
    };
    let (n1, n2) = (need(0..11), need(11..21));
    SLEN.iter()
        .enumerate()
        .filter(|&(_, &(s1, s2))| s1 >= n1 && s2 >= n2)
        .map(|(i, &(s1, s2))| (i as u8, 11 * u32::from(s1) + 10 * u32::from(s2)))
        .min_by_key(|&(_, bits)| bits)
}

/// Table 3-D.4 data. Some printed thresholds happen to be close to
/// mathematical constants (6.28 dB), which clippy would flag.
#[rustfmt::skip]
#[allow(clippy::approx_constant)]
mod ath_tables {
/// Table 3-D.4b (44.1 kHz): `(last FFT line, absthr dB)` per row.
#[rustfmt::skip]
pub const ATH_44100: [(u16, f32); 130] = [
    (1, 45.05), (2, 25.87), (3, 18.70), (4, 14.85), (5, 12.41), (6, 10.72), (7, 9.47), (8, 8.50),
    (9, 7.73), (10, 7.10), (11, 6.56), (12, 6.11), (13, 5.72), (14, 5.37), (15, 5.07), (16, 4.79),
    (17, 4.55), (18, 4.32), (19, 4.11), (20, 3.92), (21, 3.74), (22, 3.57), (23, 3.40), (24, 3.25),
    (25, 3.10), (26, 2.95), (27, 2.81), (28, 2.67), (29, 2.53), (30, 2.39), (31, 2.25), (32, 2.11),
    (33, 1.97), (34, 1.83), (35, 1.68), (36, 1.53), (37, 1.38), (38, 1.23), (39, 1.07), (40, 0.90),
    (41, 0.74), (42, 0.56), (43, 0.39), (44, 0.21), (45, 0.02), (46, -0.17), (47, -0.36), (48, -0.56),
    (50, -0.96), (52, -1.37), (54, -1.79), (56, -2.21), (58, -2.63), (60, -3.03), (62, -3.41), (64, -3.77),
    (66, -4.09), (68, -4.37), (70, -4.60), (72, -4.78), (74, -4.91), (76, -4.97), (78, -4.98), (80, -4.92),
    (82, -4.81), (84, -4.65), (86, -4.43), (88, -4.17), (90, -3.87), (92, -3.54), (94, -3.19), (96, -2.82),
    (100, -2.06), (104, -1.33), (108, -0.64), (112, -0.04), (116, 0.47), (120, 0.89), (124, 1.23), (128, 1.51),
    (132, 1.74), (136, 1.93), (140, 2.11), (144, 2.28), (148, 2.45), (152, 2.63), (156, 2.82), (160, 3.03),
    (164, 3.25), (168, 3.49), (172, 3.74), (176, 4.02), (180, 4.32), (184, 4.64), (188, 4.98), (192, 5.35),
    (200, 6.15), (208, 7.07), (216, 8.10), (224, 9.25), (232, 10.54), (240, 11.97), (248, 13.56), (256, 15.30),
    (264, 17.23), (272, 19.33), (280, 21.64), (288, 24.15), (296, 26.88), (304, 29.84), (312, 33.04), (320, 36.51),
    (328, 40.24), (336, 44.26), (344, 48.58), (352, 53.21), (360, 58.17), (368, 63.48), (376, 69.13), (384, 69.13),
    (392, 69.13), (400, 69.13), (408, 69.13), (416, 69.13), (424, 69.13), (432, 69.13), (440, 69.13), (448, 69.13),
    (456, 69.13), (464, 69.13),
];

/// Table 3-D.4c (48 kHz): `(last FFT line, absthr dB)` per row.
#[rustfmt::skip]
pub const ATH_48000: [(u16, f32); 126] = [
    (1, 42.10), (2, 24.17), (3, 17.47), (4, 13.87), (5, 11.60), (6, 10.01), (7, 8.84), (8, 7.94),
    (9, 7.22), (10, 6.62), (11, 6.12), (12, 5.70), (13, 5.33), (14, 5.00), (15, 4.71), (16, 4.45),
    (17, 4.21), (18, 4.00), (19, 3.79), (20, 3.61), (21, 3.43), (22, 3.26), (23, 3.09), (24, 2.93),
    (25, 2.78), (26, 2.63), (27, 2.47), (28, 2.32), (29, 2.17), (30, 2.02), (31, 1.86), (32, 1.71),
    (33, 1.55), (34, 1.38), (35, 1.21), (36, 1.04), (37, 0.86), (38, 0.67), (39, 0.49), (40, 0.29),
    (41, 0.09), (42, -0.11), (43, -0.32), (44, -0.54), (45, -0.75), (46, -0.97), (47, -1.20), (48, -1.43),
    (50, -1.88), (52, -2.34), (54, -2.79), (56, -3.22), (58, -3.62), (60, -3.98), (62, -4.30), (64, -4.57),
    (66, -4.77), (68, -4.91), (70, -4.98), (72, -4.97), (74, -4.90), (76, -4.76), (78, -4.55), (80, -4.29),
    (82, -3.99), (84, -3.64), (86, -3.26), (88, -2.86), (90, -2.45), (92, -2.04), (94, -1.63), (96, -1.24),
    (100, -0.51), (104, 0.12), (108, 0.64), (112, 1.06), (116, 1.39), (120, 1.66), (124, 1.88), (128, 2.08),
    (132, 2.27), (136, 2.46), (140, 2.65), (144, 2.86), (148, 3.09), (152, 3.33), (156, 3.60), (160, 3.89),
    (164, 4.20), (168, 4.54), (172, 4.91), (176, 5.31), (180, 5.73), (184, 6.18), (188, 6.67), (192, 7.19),
    (200, 8.33), (208, 9.63), (216, 11.08), (224, 12.71), (232, 14.53), (240, 16.54), (248, 18.77), (256, 21.23),
    (264, 23.94), (272, 26.90), (280, 30.14), (288, 33.67), (296, 37.51), (304, 41.67), (312, 46.17), (320, 51.04),
    (328, 56.29), (332, 61.94), (340, 68.00), (348, 68.00), (356, 68.00), (364, 68.00), (372, 68.00), (380, 68.00),
    (388, 68.00), (396, 68.00), (404, 68.00), (412, 68.00), (420, 68.00), (428, 68.00),
];

/// Table 3-D.4a (32 kHz): `(last FFT line, absthr dB)` per row.
#[rustfmt::skip]
pub const ATH_32000: [(u16, f32); 132] = [
    (1, 58.23), (2, 33.44), (3, 24.17), (4, 19.20), (5, 16.05), (6, 13.87), (7, 12.26), (8, 11.01),
    (9, 10.01), (10, 9.20), (11, 8.52), (12, 7.94), (13, 7.44), (14, 7.00), (15, 6.62), (16, 6.28),
    (17, 5.97), (18, 5.70), (19, 5.44), (20, 5.21), (21, 5.00), (22, 4.80), (23, 4.62), (24, 4.45),
    (25, 4.29), (26, 4.14), (27, 4.00), (28, 3.86), (29, 3.73), (30, 3.61), (31, 3.49), (32, 3.37),
    (33, 3.26), (34, 3.15), (35, 3.04), (36, 2.93), (37, 2.83), (38, 2.73), (39, 2.63), (40, 2.53),
    (41, 2.42), (42, 2.32), (43, 2.22), (44, 2.12), (45, 2.02), (46, 1.92), (47, 1.81), (48, 1.71),
    (50, 1.49), (52, 1.27), (54, 1.04), (56, 0.80), (58, 0.55), (60, 0.29), (62, 0.02), (64, -0.25),
    (66, -0.54), (68, -0.83), (70, -1.12), (72, -1.43), (74, -1.73), (76, -2.04), (78, -2.34), (80, -2.64),
    (82, -2.93), (84, -3.22), (86, -3.49), (88, -3.74), (90, -3.98), (92, -4.20), (94, -4.40), (96, -4.57),
    (100, -4.82), (104, -4.96), (108, -4.97), (112, -4.86), (116, -4.63), (120, -4.29), (124, -3.87), (128, -3.39),
    (132, -2.86), (136, -2.31), (140, -1.77), (144, -1.24), (148, -0.74), (152, -0.29), (156, 0.12), (160, 0.48),
    (164, 0.79), (168, 1.06), (172, 1.29), (176, 1.49), (180, 1.66), (184, 1.81), (188, 1.95), (192, 2.08),
    (200, 2.33), (208, 2.59), (216, 2.86), (224, 3.17), (232, 3.51), (240, 3.89), (248, 4.31), (256, 4.79),
    (264, 5.31), (272, 5.88), (280, 6.50), (288, 7.19), (296, 7.93), (304, 8.75), (312, 9.63), (320, 10.58),
    (328, 11.60), (336, 12.71), (344, 13.90), (352, 15.18), (360, 16.54), (368, 18.01), (376, 19.57), (384, 21.23),
    (392, 23.01), (400, 24.90), (408, 26.90), (416, 29.03), (424, 31.28), (432, 33.67), (440, 36.19), (448, 38.86),
    (456, 41.67), (464, 44.63), (472, 47.76), (480, 51.03),
];

}

/// Absolute threshold of hearing as allowed noise energy per scalefactor
/// band, in the units of our frequency lines.
///
/// Source: 11172-3 Annex D, Table 3-D.4a–c (psychoacoustic model 2), whose
/// 0 dB is "96 dB below the energy of a sine wave of amplitude ±32760", i.e.
/// a full-scale sine. original derivation of the scale: our hybrid
/// filterbank satisfies Σxr² = Σx²/288 per granule (measured in
/// `ath_reference_energy`), so a sine of amplitude A has line energy A² per
/// granule and a just-audible sine at threshold T dB has energy
/// 10^((T − 96)/10). We allow each scalefactor band (roughly a critical band)
/// that much total noise, using the lowest threshold within the band.
#[derive(Debug, Clone)]
pub struct Ath {
    /// Allowed mean noise energy per line, bands 0..=21.
    xmin: [f32; 22],
}

impl Ath {
    /// Thresholds for an MPEG-1 sample rate; `None` for LSF rates.
    pub fn new(sample_rate: u32, bands: &ScalefactorBands) -> Option<Self> {
        let table: &[(u16, f32)] = match sample_rate {
            44_100 => &ath_tables::ATH_44100,
            48_000 => &ath_tables::ATH_48000,
            32_000 => &ath_tables::ATH_32000,
            _ => return None,
        };
        // Threshold (dB) at a 1024-point FFT line. Lines above the table
        // (≈ 20 kHz, or 15 kHz at 32 kHz) take its last value.
        let at_fft_line = |line: f32| -> f32 {
            let l = line.round().max(1.0) as u16;
            table
                .iter()
                .find(|&&(hi, _)| l <= hi)
                .unwrap_or(table.last().unwrap())
                .1
        };
        let b = bands.region_bounds();
        let mut xmin = [0.0f32; 22];
        for (band, x) in xmin.iter_mut().enumerate() {
            let (lo, hi) = (b[band], b[band + 1]);
            // MDCT line l is centred on (l + 1/2) · fs/1152, which is FFT
            // line (l + 1/2) · 1024/1152.
            let min_db = (lo..hi)
                .map(|l| at_fft_line((l as f32 + 0.5) * 1024.0 / 1152.0))
                .fold(f32::INFINITY, f32::min);
            *x = 10f32.powf((min_db - 96.0) / 10.0) / (hi - lo) as f32;
        }
        Some(Self { xmin })
    }

    /// Allowed mean noise energy per line in band `b` (0..=21).
    pub fn xmin(&self, b: usize) -> f32 {
        self.xmin[b]
    }

    /// All bands' allowed distortion, as the outer loop takes it.
    pub fn xmin_all(&self) -> [f32; 22] {
        self.xmin
    }
}

/// A fully coded granule: quantized spectrum, Huffman plan and scalefactors.
#[derive(Debug, Clone)]
pub struct CodedGranule {
    pub q: QuantizedGranule,
    pub scalefac: [u8; SCALEFAC_BANDS],
    pub scalefac_compress: u8,
    pub scalefac_scale: bool,
    pub preflag: bool,
    /// Scalefactor bits (part2).
    pub part2_length: u32,
}

impl CodedGranule {
    /// A granule coded by the rate loop alone (no scalefactors).
    pub fn without_scalefactors(q: QuantizedGranule) -> Self {
        Self {
            q,
            scalefac: [0; SCALEFAC_BANDS],
            scalefac_compress: 0,
            scalefac_scale: false,
            preflag: false,
            part2_length: 0,
        }
    }

    /// `part2_3_length`: scalefactor plus Huffman bits.
    pub fn part2_3_length(&self) -> u32 {
        self.part2_length + self.q.plan.bits
    }
}

/// Amplification exponent (in powers of two) the decoder undoes in band `b`:
/// `scalefac_multiplier · (scalefac + preflag · pretab)` (11172-3 §2.4.3.4;
/// multiplier 0.5 or 1 per 13818-3 §2.5.2 `scalefac_scale`).
fn band_exponent(sf: &[u8; SCALEFAC_BANDS], b: usize, scale: bool, preflag: bool) -> f32 {
    if b >= SCALEFAC_BANDS {
        return 0.0;
    }
    let mult = if scale { 1.0 } else { 0.5 };
    mult * f32::from(sf[b] + if preflag { PRETAB[b] } else { 0 })
}

/// Mean squared quantization error per line in each band, measured in the
/// original (unamplified) domain against what the decoder reconstructs
/// (Annex C.1.5.4 §3.3, `xfsf`).
fn band_noise(xr: &[f32; GRANULE_LINES], c: &CodedGranule, bands: &ScalefactorBands) -> [f32; 22] {
    let b = bands.region_bounds();
    let step = 2f32.powf((i32::from(c.q.global_gain) - GLOBAL_GAIN_OFFSET) as f32 / 4.0);
    std::array::from_fn(|band| {
        let gain = step
            * 2f32.powf(-band_exponent(
                &c.scalefac,
                band,
                c.scalefac_scale,
                c.preflag,
            ));
        let (lo, hi) = (b[band], b[band + 1]);
        let sum: f32 = (lo..hi)
            .map(|i| {
                let rec = (c.q.ix[i].unsigned_abs() as f32).powf(4.0 / 3.0) * gain;
                (xr[i].abs() - rec).powi(2)
            })
            .sum();
        sum / (hi - lo) as f32
    })
}

/// Noise-to-threshold ratio per band, in dB (positive = audible).
fn excess_db(noise: &[f32; 22], xmin: &[f32; 22]) -> [f32; 22] {
    std::array::from_fn(|b| 10.0 * (noise[b].max(1e-30) / xmin[b].max(1e-30)).log10())
}

/// Largest noise-to-threshold excess of a granule, in dB. Lower is better.
fn worst_excess(excess: &[f32; 22]) -> f32 {
    excess.iter().copied().fold(f32::NEG_INFINITY, f32::max)
}

/// Transmitted scalefactors for a total per-band amplification `t` (in
/// scalefactor steps): `scalefac = t − preflag · pretab`.
///
/// original: `preflag` is used purely as compression. The decoder adds
/// `pretab` (Table 3-B.6) to the scalefactors when it is set, so whenever `t`
/// already contains the pretab shape, sending `t − pretab` with `preflag` = 1
/// gives the same amplification. Whichever form has the shorter part2 wins.
/// Returns `None` if neither form fits the scalefactor fields (15 for bands
/// 0–10, 7 for bands 11–20).
fn represent(t: &[u8; SCALEFAC_BANDS]) -> Option<([u8; SCALEFAC_BANDS], bool, u8, u32)> {
    let plain = choose_scalefac_compress(t).map(|(c, bits)| (*t, false, c, bits));
    let with_pretab = t
        .iter()
        .zip(&PRETAB)
        .all(|(&v, &p)| v >= p)
        .then(|| std::array::from_fn(|b| t[b] - PRETAB[b]))
        .and_then(|sf| choose_scalefac_compress(&sf).map(|(c, bits)| (sf, true, c, bits)));
    match (plain, with_pretab) {
        (Some(a), Some(b)) => Some(if b.3 < a.3 { b } else { a }),
        (a, b) => a.or(b),
    }
}

/// Outer-loop iteration cap. original: a guard on encoding time; the loop
/// normally ends earlier on convergence (see `outer_loop`).
const MAX_OUTER_ITERATIONS: usize = 32;

/// Outer (distortion control) loop, Annex C.1.5.4 §3, long blocks.
///
/// Repeatedly quantizes with the rate loop, measures each band's noise
/// against the allowed distortion `xmin` (mean noise energy per line, bands
/// 0..=21), and amplifies (raises the scalefactor of) bands.
///
/// As in §3.5, every band whose noise exceeds the allowed distortion is
/// amplified by one step per iteration. original refinements: the smallest
/// amplification is kept at zero, because amplifying every band equally is
/// undone by the rate loop raising `global_gain` and only costs scalefactor
/// bits; consequently the loop stops as soon as an iteration would change
/// nothing (e.g. every band is over the threshold, typical at 128 kbps with
/// the absolute threshold alone), as well as when all bands are under it,
/// after three iterations without improvement, or at the field-size limit.
/// Of all iterations, the one with the smallest worst-band noise-to-threshold
/// ratio is kept, rather than only the last saved one (§3.1).
///
/// Evaluation (2026-10-19, `eval/results/m4-annex.json` vs `m3-baseline`):
/// with the absolute threshold as the allowed distortion this loop does not
/// beat the rate loop alone (dev mean ViSQOL 4.38/4.60/4.64 vs 4.39/4.63/4.72
/// at 128/192/320 kbps). Gaining only bands over the threshold raises the
/// global gain, which moves noise into the 12–22 kHz bands where the
/// threshold is high. Rejected variants: equalising the excess over the
/// threshold (vocal quartet 128 kbps: 3.38 → 2.89) and keeping the
/// iteration with the least total excess (violin 320 kbps: 4.71 → 4.17).
/// With psychoacoustic model 2's masking threshold (M5, run
/// `m5b-q0.01-over-128`) the same loop lifts the 128 kbps dev mean from 4.39
/// to 4.48. Amplifying only the bands within 1.5 dB of the worst
/// noise-to-mask ratio (minimax) scored the same (4.48, run
/// `m5b-q0.01-worst-128`), so the Annex rule is kept.
///
/// It runs with `scalefac_scale` = 0 (√2 steps) and, if that hits the field
/// limit, again with `scalefac_scale` = 1 (2× steps); the better is kept.
pub fn outer_loop(
    xr: &[f32; GRANULE_LINES],
    bands: &ScalefactorBands,
    xmin: &[f32; 22],
    max_bits: u32,
) -> CodedGranule {
    let (fine, fine_score, hit_limit) = outer_loop_with_scale(xr, bands, xmin, max_bits, false);
    if !hit_limit {
        return fine;
    }
    let (coarse, coarse_score, _) = outer_loop_with_scale(xr, bands, xmin, max_bits, true);
    if coarse_score < fine_score {
        coarse
    } else {
        fine
    }
}

/// One run of the outer loop at a fixed `scalefac_scale`. Returns the best
/// iteration, its worst-band excess, and whether the field limit was hit.
fn outer_loop_with_scale(
    xr: &[f32; GRANULE_LINES],
    bands: &ScalefactorBands,
    xmin: &[f32; 22],
    max_bits: u32,
    scale: bool,
) -> (CodedGranule, f32, bool) {
    let bounds = bands.region_bounds();
    let mut t = [0u8; SCALEFAC_BANDS];
    let mut hint = None;
    let mut best: Option<(CodedGranule, f32)> = None;
    let mut stall = 0;
    let mut hit_limit = false;
    for _ in 0..MAX_OUTER_ITERATIONS {
        let floor = t.iter().copied().min().unwrap_or(0);
        t.iter_mut().for_each(|v| *v -= floor);
        let Some((scalefac, preflag, scalefac_compress, part2_length)) = represent(&t) else {
            hit_limit = true;
            break;
        };
        // Amplify each band by 2^exponent before quantizing; the decoder
        // divides it back out (Annex C.1.5.4 §3).
        let mut amp = *xr;
        for band in 0..SCALEFAC_BANDS {
            let g = 2f32.powf(band_exponent(&scalefac, band, scale, preflag));
            for v in &mut amp[bounds[band]..bounds[band + 1]] {
                *v *= g;
            }
        }
        let q = rate_loop_from(&amp, bands, max_bits.saturating_sub(part2_length), hint);
        hint = Some(q.global_gain);
        let coded = CodedGranule {
            q,
            scalefac,
            scalefac_compress,
            scalefac_scale: scale,
            preflag,
            part2_length,
        };
        let excess = excess_db(&band_noise(xr, &coded, bands), xmin);
        let score = worst_excess(&excess);
        if best.as_ref().is_none_or(|(_, s)| score < *s - 0.01) {
            best = Some((coded, score));
            stall = 0;
        } else {
            stall += 1;
            if stall >= 3 {
                break;
            }
        }
        let worst = excess[..SCALEFAC_BANDS]
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        if worst <= 0.0 {
            break; // every band with a scalefactor is under the threshold
        }
        let before = t;
        for (v, &e) in t.iter_mut().zip(&excess) {
            if e > 0.0 {
                *v += 1;
            }
        }
        // Amplifying every band equally is a no-op once normalised.
        let floor = t.iter().copied().min().unwrap_or(0);
        if t.iter().zip(&before).all(|(&a, &b)| a - floor == b) {
            break;
        }
    }
    let (coded, score) = best.expect("first iteration always fits (all-zero scalefactors)");
    (coded, score, hit_limit)
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

    #[test]
    fn ath_reference_energy() {
        // The Ath scale assumes Σxr² = Σx²/288 per granule through our
        // hybrid filterbank, i.e. a sine of amplitude A has line energy A².
        use crate::mdct::{GRANULE_SAMPLES, HybridFilterbank};
        for (f, amp) in [(1000.0f64, 1.0f32), (5000.0, 0.25), (12_000.0, 0.5)] {
            let mut fb = HybridFilterbank::new();
            let mut xr = [0.0f32; GRANULE_LINES];
            let mut energy = 0.0f64;
            let granules = 20;
            for g in 0..granules {
                let pcm: [f32; GRANULE_SAMPLES] = std::array::from_fn(|i| {
                    let n = (g * GRANULE_SAMPLES + i) as f64;
                    amp * (2.0 * std::f64::consts::PI * f * n / 44_100.0).sin() as f32
                });
                fb.granule(&pcm, crate::mdct::Block::Normal, &mut xr);
                if g >= 4 {
                    energy += xr.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>();
                }
            }
            let per_granule = energy / (granules - 4) as f64;
            let expect = f64::from(amp).powi(2);
            assert!(
                (per_granule / expect - 1.0).abs() < 0.02,
                "{f} Hz: {per_granule} vs {expect}"
            );
        }
    }

    #[test]
    fn ath_spot_values() {
        // 44.1 kHz band 0 spans FFT lines ~0.4–3.1, whose lowest threshold
        // in Table 3-D.4b is 18.70 dB (line 3); band 0 has 4 lines.
        let bands = ScalefactorBands::long(44_100).unwrap();
        let ath = Ath::new(44_100, &bands).unwrap();
        let expect = 10f32.powf((18.70 - 96.0) / 10.0) / 4.0;
        assert!((ath.xmin(0) / expect - 1.0).abs() < 1e-4);
        // Mid bands are far more sensitive than the top band.
        assert!(ath.xmin(10) < ath.xmin(21) / 1000.0);
        assert!(Ath::new(22_050, &ScalefactorBands::long(22_050).unwrap()).is_none());
    }

    #[test]
    fn scalefac_compress_selection() {
        let mut sf = [0u8; SCALEFAC_BANDS];
        assert_eq!(choose_scalefac_compress(&sf), Some((0, 0)));
        sf[3] = 5; // needs slen1 = 3 -> index 4 (3, 0): 33 bits
        assert_eq!(choose_scalefac_compress(&sf), Some((4, 33)));
        sf[15] = 1; // slen2 >= 1 -> (3, 1), index 11: 43 bits
        assert_eq!(choose_scalefac_compress(&sf), Some((11, 43)));
        sf[0] = 15;
        sf[20] = 7; // (4, 3), index 15
        assert_eq!(choose_scalefac_compress(&sf), Some((15, 74)));
        sf[20] = 8; // no slen2 can carry 8
        assert_eq!(choose_scalefac_compress(&sf), None);
    }

    #[test]
    fn preflag_used_when_cheaper() {
        // t equal to pretab: sending zeros with preflag costs 0 bits.
        let (sf, preflag, compress, bits) = represent(&PRETAB).unwrap();
        assert!(preflag);
        assert_eq!((sf, compress, bits), ([0; SCALEFAC_BANDS], 0, 0));
        // A band below pretab rules preflag out.
        let mut t = PRETAB;
        t[17] = 0;
        assert!(!represent(&t).unwrap().1);
        // Too large for any field either way.
        let mut t = [0u8; SCALEFAC_BANDS];
        t[12] = 11;
        assert!(represent(&t).is_none());
    }

    #[test]
    fn outer_loop_shapes_noise_and_matches_decoder() {
        use crate::refdec::requantize_long;
        let bands = ScalefactorBands::long(44_100).unwrap();
        let ath = Ath::new(44_100, &bands).unwrap();
        // Loud low band, very quiet high band: a budget that leaves the low
        // band over the threshold makes the loop amplify it.
        let mut xr = [0.0f32; GRANULE_LINES];
        for (i, x) in xr.iter_mut().enumerate() {
            let level = if i < 100 { 0.05 } else { 1e-5 };
            *x = level * ((i as f32 * 1.7).sin() + 0.3);
        }
        let c = outer_loop(&xr, &bands, &ath.xmin_all(), 1500);
        assert!(c.part2_3_length() <= 1500);
        assert!(
            c.scalefac.iter().any(|&v| v > 0) || c.preflag,
            "no shaping happened"
        );
        let (slen1, slen2) = SLEN[usize::from(c.scalefac_compress)];
        assert_eq!(
            c.part2_length,
            11 * u32::from(slen1) + 10 * u32::from(slen2)
        );
        // The decoder's reconstruction (independent formula) must match the
        // noise the encoder measured.
        let rec = requantize_long(
            &c.q.ix,
            c.q.global_gain,
            &c.scalefac,
            c.preflag,
            c.scalefac_scale,
            bands.region_bounds(),
        );
        let ours = band_noise(&xr, &c, &bands);
        let b = bands.region_bounds();
        for band in 0..22 {
            let (lo, hi) = (b[band], b[band + 1]);
            let n: f64 = (lo..hi)
                .map(|i| (f64::from(xr[i]) - rec[i]).powi(2))
                .sum::<f64>()
                / (hi - lo) as f64;
            let m = f64::from(ours[band]);
            assert!(
                (n - m).abs() <= 1e-3 * n.max(m) + 1e-20,
                "band {band}: decoder {n} vs encoder {m}"
            );
        }
    }
}
