//! Mid/side stereo: the transform and the per-frame decision.
//!
//! Sources (see PROVENANCE.md):
//! - ISO/IEC 11172-3 §2.4.3.4 (MS_stereo mode): the decoder reconstructs
//!   `L = (M + S)/√2`, `R = (M − S)/√2` over the whole spectrum when
//!   intensity stereo is off, M in the first channel and S in the second;
//!   both channels of a granule must have the same block type.
//! - 11172-3 CD rev 1, Annex G.2: the encoder transmits `M = (L + R)/√2`,
//!   `S = (L − R)/√2`, deciding from the channels' FFT energies. The switching
//!   criterion's formula is unreadable in every copy we have (broken
//!   equation fields), so the decision below is our own.
//!
//! Intensity stereo is not used.

use crate::mdct::GRANULE_LINES;

/// Replace `left`/`right` by mid/side in place (Annex G.2 matrix; the exact
/// inverse of the decoder's §2.4.3.4 matrix).
pub fn to_mid_side(left: &mut [f32; GRANULE_LINES], right: &mut [f32; GRANULE_LINES]) {
    for (l, r) in left.iter_mut().zip(right.iter_mut()) {
        let (m, s) = (
            (*l + *r) * std::f32::consts::FRAC_1_SQRT_2,
            (*l - *r) * std::f32::consts::FRAC_1_SQRT_2,
        );
        *l = m;
        *r = s;
    }
}

/// Allowed distortion for both the mid and the side channel, per band, from
/// the left and right channels' allowed distortions.
///
/// original: the decoder's matrix gives the left channel the noise
/// `(n_M + n_S)/√2`; for independent mid and side noise its power is
/// `(N_M + N_S)/2`, and the same for the right channel. With
/// `N_M, N_S ≤ min(X_L, X_R)` both reconstructed channels therefore stay
/// within their own allowed distortion, whatever the noise in the other
/// channel. This ignores binaural masking release (noise that is
/// correlated differently from the signal can be more audible), which is
/// conservative only in the sense of the channel thresholds themselves.
pub fn mid_side_xmin(left: &[f32], right: &[f32]) -> Vec<f32> {
    left.iter().zip(right).map(|(l, r)| l.min(*r)).collect()
}

/// original: code the frame as mid/side when that is estimated to need
/// fewer bits for the same allowed distortion. The estimates are
/// [`crate::quantize::demand_bits`] summed over both granules and channels,
/// with the [`mid_side_xmin`] thresholds for M/S; the decision is per frame
/// because `mode_extension` is in the frame header (§2.4.2.3).
pub fn prefer_mid_side(lr_bits: f32, ms_bits: f32) -> bool {
    ms_bits < lr_bits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mdct::Block;
    use crate::quantize::{ScalefactorBands, demand_bits};

    #[test]
    fn transform_matches_decoder_inverse() {
        let mut l = [0.0f32; GRANULE_LINES];
        let mut r = [0.0f32; GRANULE_LINES];
        for i in 0..GRANULE_LINES {
            l[i] = (i as f32 * 0.37).sin();
            r[i] = (i as f32 * 0.11).cos() * 0.5;
        }
        let (l0, r0) = (l, r);
        to_mid_side(&mut l, &mut r);
        let k = std::f32::consts::FRAC_1_SQRT_2;
        for i in 0..GRANULE_LINES {
            // §2.4.3.4: L = (M + S)/√2, R = (M − S)/√2.
            assert!(((l[i] + r[i]) * k - l0[i]).abs() < 1e-6);
            assert!(((l[i] - r[i]) * k - r0[i]).abs() < 1e-6);
        }
    }

    /// Pseudo-random spectrum (xorshift), deterministic.
    fn noise(seed: u32) -> [f32; GRANULE_LINES] {
        let mut s = seed;
        std::array::from_fn(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32) - 0.5
        })
    }

    fn decide(l: [f32; GRANULE_LINES], r: [f32; GRANULE_LINES]) -> bool {
        let bands = ScalefactorBands::new(44_100).unwrap();
        let layout = bands.layout(Block::Normal);
        // Allowed distortion 30 dB below each channel's band energy.
        let xmin = |x: &[f32; GRANULE_LINES]| -> Vec<f32> {
            layout
                .bands
                .iter()
                .map(|&(lo, hi, _)| {
                    1e-3 * x[lo..hi].iter().map(|v| v * v).sum::<f32>() / (hi - lo) as f32
                })
                .collect()
        };
        let (xl, xr) = (xmin(&l), xmin(&r));
        let lr = demand_bits(&l, &layout, &xl) + demand_bits(&r, &layout, &xr);
        let ms_x = mid_side_xmin(&xl, &xr);
        let (mut m, mut s) = (l, r);
        to_mid_side(&mut m, &mut s);
        let ms = demand_bits(&m, &layout, &ms_x) + demand_bits(&s, &layout, &ms_x);
        prefer_mid_side(lr, ms)
    }

    #[test]
    fn mono_compatible_material_picks_mid_side() {
        // Nearly identical channels: the side channel is almost empty.
        let a = noise(1);
        let b = noise(2);
        let r: [f32; GRANULE_LINES] = std::array::from_fn(|i| a[i] + 0.01 * b[i]);
        assert!(decide(a, r));
    }

    #[test]
    fn uncorrelated_material_stays_left_right() {
        assert!(!decide(noise(1), noise(2)));
        // One channel much louder: M and S both carry it, and inherit the
        // quiet channel's threshold, so L/R is cheaper.
        let a = noise(3);
        let quiet: [f32; GRANULE_LINES] = noise(4).map(|v| 0.01 * v);
        assert!(!decide(a, quiet));
    }
}
