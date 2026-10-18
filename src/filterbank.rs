//! 32-band polyphase analysis filterbank.
//!
//! **Milestone:** M2. See docs/PLAN.md.
//!
//! **Allowed sources:** ISO/IEC 11172-3 §2.4.3.2 and Annex C.1.3 (analysis window coefficients: Table C.1).
//!
//! **Must be tested by:** Test against a slow direct evaluation of the matrixing formula; sines must land in the right subband.
//!
//! Clean-room reminder: derive from the sources above only. Cite clauses in
//! comments and log every source in PROVENANCE.md (CLEANROOM.md).
//!
//! Implements 11172-3 Annex C.1.3, Figure 3-C.1 "Analysis subband filter
//! flow chart", as printed in the CD 11172-3 rev 1 text (see PROVENANCE.md).

use std::f64::consts::PI;

/// Number of subbands, and new input samples consumed per call.
pub const SUBBANDS: usize = 32;
/// Length of the analysis window and input history.
pub const WINDOW_LEN: usize = 512;

/// Analysis window, Table 3-C.1, as integers: `C[i] = WINDOW_Q21[i] / 2^21`.
///
/// The table prints each `C[i]` to nine decimals. Every entry is an exact
/// multiple of 2^-21 (checked when transcribing; 512 of 512), so storing the
/// integer numerators is lossless. The synthesis window of Table 3-B.3 is
/// the same numerators over 2^16, i.e. `D[i] = 32 · C[i]` for every `i`.
/// Values were cross-checked entry by entry against ETSI TS 103 466 V1.2.1
/// Table C.1, which reproduces the same window.
#[rustfmt::skip]
pub const WINDOW_Q21: [i32; WINDOW_LEN] = [
    0, -1, -1, -1, -1, -1, -1, -2,
    -2, -2, -2, -3, -3, -4, -4, -5,
    -5, -6, -7, -7, -8, -9, -10, -11,
    -13, -14, -16, -17, -19, -21, -24, -26,
    -29, -31, -35, -38, -41, -45, -49, -53,
    -58, -63, -68, -73, -79, -85, -91, -97,
    -104, -111, -117, -125, -132, -139, -147, -154,
    -161, -169, -176, -183, -190, -196, -202, -208,
    213, 218, 222, 225, 227, 228, 228, 227,
    224, 221, 215, 208, 200, 189, 177, 163,
    146, 127, 106, 83, 57, 29, -2, -36,
    -72, -111, -153, -197, -244, -294, -347, -401,
    -459, -519, -581, -645, -711, -779, -848, -919,
    -991, -1064, -1137, -1210, -1283, -1356, -1428, -1498,
    -1567, -1634, -1698, -1759, -1817, -1870, -1919, -1962,
    -2001, -2032, -2057, -2075, -2085, -2087, -2080, -2063,
    2037, 2000, 1952, 1893, 1822, 1739, 1644, 1535,
    1414, 1280, 1131, 970, 794, 605, 402, 185,
    -45, -288, -545, -814, -1095, -1388, -1692, -2006,
    -2330, -2663, -3004, -3351, -3705, -4063, -4425, -4788,
    -5153, -5517, -5879, -6237, -6589, -6935, -7271, -7597,
    -7910, -8209, -8491, -8755, -8998, -9219, -9416, -9585,
    -9727, -9838, -9916, -9959, -9966, -9935, -9863, -9750,
    -9592, -9389, -9139, -8840, -8492, -8092, -7640, -7134,
    6574, 5959, 5288, 4561, 3776, 2935, 2037, 1082,
    70, -998, -2122, -3300, -4533, -5818, -7154, -8540,
    -9975, -11455, -12980, -14548, -16155, -17799, -19478, -21189,
    -22929, -24694, -26482, -28289, -30112, -31947, -33791, -35640,
    -37489, -39336, -41176, -43006, -44821, -46617, -48390, -50137,
    -51853, -53534, -55178, -56778, -58333, -59838, -61289, -62684,
    -64019, -65290, -66494, -67629, -68692, -69679, -70590, -71420,
    -72169, -72835, -73415, -73908, -74313, -74630, -74856, -74992,
    75038, 74992, 74856, 74630, 74313, 73908, 73415, 72835,
    72169, 71420, 70590, 69679, 68692, 67629, 66494, 65290,
    64019, 62684, 61289, 59838, 58333, 56778, 55178, 53534,
    51853, 50137, 48390, 46617, 44821, 43006, 41176, 39336,
    37489, 35640, 33791, 31947, 30112, 28289, 26482, 24694,
    22929, 21189, 19478, 17799, 16155, 14548, 12980, 11455,
    9975, 8540, 7154, 5818, 4533, 3300, 2122, 998,
    -70, -1082, -2037, -2935, -3776, -4561, -5288, -5959,
    6574, 7134, 7640, 8092, 8492, 8840, 9139, 9389,
    9592, 9750, 9863, 9935, 9966, 9959, 9916, 9838,
    9727, 9585, 9416, 9219, 8998, 8755, 8491, 8209,
    7910, 7597, 7271, 6935, 6589, 6237, 5879, 5517,
    5153, 4788, 4425, 4063, 3705, 3351, 3004, 2663,
    2330, 2006, 1692, 1388, 1095, 814, 545, 288,
    45, -185, -402, -605, -794, -970, -1131, -1280,
    -1414, -1535, -1644, -1739, -1822, -1893, -1952, -2000,
    2037, 2063, 2080, 2087, 2085, 2075, 2057, 2032,
    2001, 1962, 1919, 1870, 1817, 1759, 1698, 1634,
    1567, 1498, 1428, 1356, 1283, 1210, 1137, 1064,
    991, 919, 848, 779, 711, 645, 581, 519,
    459, 401, 347, 294, 244, 197, 153, 111,
    72, 36, 2, -29, -57, -83, -106, -127,
    -146, -163, -177, -189, -200, -208, -215, -221,
    -224, -227, -228, -228, -227, -225, -222, -218,
    213, 208, 202, 196, 190, 183, 176, 169,
    161, 154, 147, 139, 132, 125, 117, 111,
    104, 97, 91, 85, 79, 73, 68, 63,
    58, 53, 49, 45, 41, 38, 35, 31,
    29, 26, 24, 21, 19, 17, 16, 14,
    13, 11, 10, 9, 8, 7, 7, 6,
    5, 5, 4, 4, 3, 3, 2, 2,
    2, 2, 1, 1, 1, 1, 1, 1,
];

/// `C[i]` as a float.
pub fn analysis_window(i: usize) -> f64 {
    f64::from(WINDOW_Q21[i]) / f64::from(1 << 21)
}

/// Per-channel analysis filterbank state.
#[derive(Debug, Clone)]
pub struct AnalysisFilterbank {
    /// Input history `X`, most recent sample at index 0 (Figure 3-C.1).
    x: [f32; WINDOW_LEN],
    window: [f32; WINDOW_LEN],
    /// `M[i][k] = cos((2i + 1)(k − 16)π / 64)`, Annex C.1.3.
    matrix: Box<[[f32; 64]; SUBBANDS]>,
}

impl Default for AnalysisFilterbank {
    fn default() -> Self {
        Self::new()
    }
}

impl AnalysisFilterbank {
    pub fn new() -> Self {
        Self {
            x: [0.0; WINDOW_LEN],
            window: std::array::from_fn(|i| analysis_window(i) as f32),
            matrix: Box::new(std::array::from_fn(|i| {
                std::array::from_fn(|k| {
                    ((2 * i + 1) as f64 * (k as f64 - 16.0) * PI / 64.0).cos() as f32
                })
            })),
        }
    }

    /// Consume 32 input samples (oldest first) and produce one sample for
    /// each of the 32 subbands, lowest subband first.
    pub fn process(&mut self, input: &[f32; SUBBANDS], out: &mut [f32; SUBBANDS]) {
        // Shift the history by 32, then insert the new samples with the most
        // recent at X[0]: the flow chart fills X[31] down to X[0] in input
        // order.
        self.x.copy_within(..WINDOW_LEN - SUBBANDS, SUBBANDS);
        for (dst, &s) in self.x[..SUBBANDS].iter_mut().zip(input.iter().rev()) {
            *dst = s;
        }

        // Window (Z_i = C_i · X_i) and partial calculation
        // Y_i = Σ_{j=0}^{7} Z_{i+64j}, for i = 0..63.
        let mut y = [0.0f32; 64];
        for (i, yi) in y.iter_mut().enumerate() {
            *yi = (0..8)
                .map(|j| self.window[i + 64 * j] * self.x[i + 64 * j])
                .sum();
        }

        // Matrixing: S_i = Σ_{k=0}^{63} M_ik · Y_k.
        for (s, row) in out.iter_mut().zip(self.matrix.iter()) {
            *s = row.iter().zip(&y).map(|(m, y)| m * y).sum();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_table_properties() {
        // Spot values from Table 3-C.1: C[1] = -0.000000477, C[256] = 0.035780907.
        assert!((analysis_window(1) + 0.000000477).abs() < 5e-10);
        assert!((analysis_window(256) - 0.035780907).abs() < 5e-10);
        assert_eq!(WINDOW_Q21[0], 0);
        // Observed in the transcribed table (a transcription-error check, not
        // a property the spec states): C[512 - i] = -C[i] for 0 < i < 256,
        // except at i = 64, 128, 192 where the two entries are equal.
        for i in 1..256 {
            let expect = if i % 64 == 0 {
                WINDOW_Q21[i]
            } else {
                -WINDOW_Q21[i]
            };
            assert_eq!(WINDOW_Q21[512 - i], expect, "i={i}");
        }
    }

    /// Direct evaluation of Figure 3-C.1 in f64 from the full input history.
    fn direct(history: &[f64], t: usize) -> [f64; SUBBANDS] {
        // X_i is the input sample i positions before the newest one at t.
        let x = |i: usize| if i <= t { history[t - i] } else { 0.0 };
        std::array::from_fn(|sb| {
            (0..64)
                .map(|k| {
                    let m = ((2 * sb + 1) as f64 * (k as f64 - 16.0) * PI / 64.0).cos();
                    let y: f64 = (0..8)
                        .map(|j| analysis_window(k + 64 * j) * x(k + 64 * j))
                        .sum();
                    m * y
                })
                .sum()
        })
    }

    #[test]
    fn matches_direct_evaluation() {
        // original: 64-bit LCG (Knuth MMIX constants), top 24 bits -> [-1, 1).
        let mut seed = 7u64;
        let signal: Vec<f64> = (0..32 * 40)
            .map(|_| {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                ((seed >> 40) as f64 / f64::from(1 << 23)) - 1.0
            })
            .collect();
        let mut fb = AnalysisFilterbank::new();
        let mut out = [0.0f32; SUBBANDS];
        for (n, chunk) in signal.chunks_exact(SUBBANDS).enumerate() {
            let input: [f32; SUBBANDS] = std::array::from_fn(|i| chunk[i] as f32);
            fb.process(&input, &mut out);
            let expect = direct(&signal, n * SUBBANDS + SUBBANDS - 1);
            for sb in 0..SUBBANDS {
                assert!(
                    (f64::from(out[sb]) - expect[sb]).abs() < 1e-5,
                    "block {n} sb {sb}: {} vs {}",
                    out[sb],
                    expect[sb]
                );
            }
        }
    }

    #[test]
    fn sines_land_in_their_subband() {
        // A sine at the centre of subband sb, (sb + 1/2) · fs/64, should put
        // nearly all its energy in that subband.
        for sb in [0, 1, 5, 16, 30, 31] {
            let w = (sb as f64 + 0.5) * PI / 32.0; // radians per sample
            let mut fb = AnalysisFilterbank::new();
            let mut out = [0.0f32; SUBBANDS];
            let mut energy = [0.0f64; SUBBANDS];
            for block in 0..64 {
                let input: [f32; SUBBANDS] =
                    std::array::from_fn(|i| (w * (block * SUBBANDS + i) as f64).sin() as f32);
                fb.process(&input, &mut out);
                if block >= 16 {
                    for (e, &s) in energy.iter_mut().zip(&out) {
                        *e += f64::from(s) * f64::from(s);
                    }
                }
            }
            let total: f64 = energy.iter().sum();
            assert!(energy[sb] / total > 0.999, "sb {sb}: {:?}", energy);
        }
    }
}
