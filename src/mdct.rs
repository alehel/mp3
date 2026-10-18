//! MDCT with window switching, plus encoder-side alias reduction.
//!
//! **Milestone:** M2 (long blocks), M6 (short/start/stop). See docs/PLAN.md.
//!
//! **Allowed sources:** ISO/IEC 11172-3 §2.4.3.4 (window shapes, IMDCT definition, alias reduction coefficients); Princen & Bradley 1986 (TDAC).
//!
//! **Must be tested by:** Implement a test-only IMDCT and verify perfect reconstruction (TDAC) for every window sequence.
//!
//! Clean-room reminder: derive from the sources above only. Cite clauses in
//! comments and log every source in PROVENANCE.md (CLEANROOM.md).
//!
//! Status: long blocks (`block_type` 0) only. Alias reduction is still
//! missing: its coefficients are in Annex B Table 3-B.9, which the project's
//! copy of the standard does not include.

use std::f64::consts::PI;

/// Polyphase subbands per granule.
pub const SUBBANDS: usize = 32;
/// Subband samples per granule, and MDCT lines per subband (long blocks).
pub const LINES: usize = 18;
/// Frequency lines per granule.
pub const GRANULE_LINES: usize = SUBBANDS * LINES;
/// Long-block MDCT length: 18 new samples plus 18 overlapped.
const N: usize = 2 * LINES;

/// One granule of polyphase output, time-major: `[time][subband]`.
pub type SubbandGranule = [[f32; SUBBANDS]; LINES];

/// Long-block window (`block_type` 0), 11172-3 §2.4.3.4 "Windowing" a):
/// `w_i = sin(π/36 · (i + 1/2))`. It satisfies the Princen–Bradley condition
/// `w_i² + w_{i+18}² = 1`, which TDAC needs.
fn long_window() -> [f32; N] {
    std::array::from_fn(|i| (PI / N as f64 * (i as f64 + 0.5)).sin() as f32)
}

/// Forward MDCT matrix for n = 36, the transpose of the IMDCT kernel in
/// 11172-3 §2.4.3.4: `cos(π/(2n) · (2i + 1 + n/2) · (2k + 1))`, scaled by 4/n.
///
/// original: the decoder's IMDCT is unscaled. Windowed by `w` on both sides
/// and overlap-added, IMDCT(MDCT(x)) gives `n/4 · x` (checked by the TDAC
/// test). Scaling the forward transform by 4/n makes a conforming decoder
/// reconstruct our subband samples at unit gain. Computed in f64, stored f32.
fn mdct_matrix() -> Box<[[f32; N]; LINES]> {
    let n = N as f64;
    Box::new(std::array::from_fn(|k| {
        std::array::from_fn(|i| {
            let arg = PI / (2.0 * n) * (2.0 * i as f64 + 1.0 + n / 2.0) * (2.0 * k as f64 + 1.0);
            (4.0 / n * arg.cos()) as f32
        })
    }))
}

/// Per-channel MDCT state.
#[derive(Debug, Clone)]
pub struct Mdct {
    /// Previous granule's subband samples (after frequency inversion),
    /// `[subband][time]`: the first half of the next 36-sample block.
    prev: [[f32; LINES]; SUBBANDS],
    window: [f32; N],
    matrix: Box<[[f32; N]; LINES]>,
}

impl Default for Mdct {
    fn default() -> Self {
        Self::new()
    }
}

impl Mdct {
    pub fn new() -> Self {
        Self {
            prev: [[0.0; LINES]; SUBBANDS],
            window: long_window(),
            matrix: mdct_matrix(),
        }
    }

    /// Transform one granule of subband samples into 576 frequency lines,
    /// ordered as the decoder expects: line `18 · sb + k` (11172-3 §2.4.3.4,
    /// "each 18 into one transform block").
    ///
    /// The lines describe the 36-sample block ending with this granule, so a
    /// decoder reproduces the subband samples one granule (18 samples per
    /// subband) late.
    pub fn granule(&mut self, input: &SubbandGranule, out: &mut [f32; GRANULE_LINES]) {
        for sb in 0..SUBBANDS {
            let mut cur: [f32; LINES] = std::array::from_fn(|t| input[t][sb]);
            // 11172-3 §2.4.3.4, "Synthesis filterbank": the decoder negates
            // every second sample of every second subband after the IMDCT to
            // undo the polyphase filterbank's frequency inversion. Negation
            // is its own inverse, so the encoder applies the same step before
            // the MDCT. LINES is even, so parity is stable across granules.
            if sb % 2 == 1 {
                for s in cur.iter_mut().skip(1).step_by(2) {
                    *s = -*s;
                }
            }

            let block: [f32; N] = std::array::from_fn(|i| {
                let x = if i < LINES {
                    self.prev[sb][i]
                } else {
                    cur[i - LINES]
                };
                x * self.window[i]
            });
            let lines = &mut out[sb * LINES..(sb + 1) * LINES];
            for (line, row) in lines.iter_mut().zip(self.matrix.iter()) {
                *line = row.iter().zip(&block).map(|(c, z)| c * z).sum();
            }
            self.prev[sb] = cur;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only decoder path for one subband: IMDCT exactly as written in
    /// 11172-3 §2.4.3.4 (unscaled, f64), long window, overlap-add, then the
    /// frequency inversion for odd subbands.
    struct Imdct {
        overlap: [[f64; LINES]; SUBBANDS],
    }

    impl Imdct {
        fn new() -> Self {
            Self {
                overlap: [[0.0; LINES]; SUBBANDS],
            }
        }

        fn granule(&mut self, lines: &[f32; GRANULE_LINES]) -> SubbandGranule {
            let n = N as f64;
            let mut out = [[0.0f32; SUBBANDS]; LINES];
            for sb in 0..SUBBANDS {
                let x = &lines[sb * LINES..(sb + 1) * LINES];
                let z: Vec<f64> = (0..N)
                    .map(|i| {
                        let xi: f64 = (0..LINES)
                            .map(|k| {
                                let arg = PI / (2.0 * n)
                                    * (2.0 * i as f64 + 1.0 + n / 2.0)
                                    * (2.0 * k as f64 + 1.0);
                                f64::from(x[k]) * arg.cos()
                            })
                            .sum();
                        xi * (PI / 36.0 * (i as f64 + 0.5)).sin()
                    })
                    .collect();
                for t in 0..LINES {
                    let mut s = z[t] + self.overlap[sb][t];
                    if sb % 2 == 1 && t % 2 == 1 {
                        s = -s;
                    }
                    out[t][sb] = s as f32;
                    self.overlap[sb][t] = z[t + LINES];
                }
            }
            out
        }
    }

    /// Deterministic pseudo-random subband signal in [-1, 1).
    fn noise(seed: &mut u64) -> f32 {
        // original: 64-bit LCG (Knuth MMIX constants), top 24 bits.
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*seed >> 40) as f32 / (1u64 << 23) as f32) - 1.0
    }

    #[test]
    fn window_satisfies_princen_bradley() {
        let w = long_window();
        for i in 0..LINES {
            let s = w[i] * w[i] + w[i + LINES] * w[i + LINES];
            assert!((s - 1.0).abs() < 1e-6, "i={i}: {s}");
        }
    }

    #[test]
    fn tdac_perfect_reconstruction() {
        let mut mdct = Mdct::new();
        let mut imdct = Imdct::new();
        let mut seed = 1;
        let mut lines = [0.0f32; GRANULE_LINES];
        let mut prev_input: Option<SubbandGranule> = None;
        let mut max_err = 0.0f32;
        for _ in 0..20 {
            let input: SubbandGranule =
                std::array::from_fn(|_| std::array::from_fn(|_| noise(&mut seed)));
            mdct.granule(&input, &mut lines);
            let rec = imdct.granule(&lines);
            // One granule of delay.
            if let Some(expect) = prev_input {
                for t in 0..LINES {
                    for sb in 0..SUBBANDS {
                        max_err = max_err.max((rec[t][sb] - expect[t][sb]).abs());
                    }
                }
            }
            prev_input = Some(input);
        }
        assert!(max_err < 1e-5, "max reconstruction error {max_err}");
    }

    #[test]
    fn impulse_matches_formula() {
        // Known values: a single sample of 1.0 at time t0 of the current
        // granule in subband 0 sits at block index i = 18 + t0, so line k is
        // (4/36) · w_i · cos(π/72 · (2i + 19) · (2k + 1)).
        let t0 = 5;
        let mut input = [[0.0f32; SUBBANDS]; LINES];
        input[t0][0] = 1.0;
        let mut out = [0.0f32; GRANULE_LINES];
        Mdct::new().granule(&input, &mut out);
        let i = (LINES + t0) as f64;
        let w = (PI / 36.0 * (i + 0.5)).sin();
        for (k, &line) in out[..LINES].iter().enumerate() {
            let expect =
                4.0 / 36.0 * w * (PI / 72.0 * (2.0 * i + 19.0) * (2.0 * k as f64 + 1.0)).cos();
            assert!((f64::from(line) - expect).abs() < 1e-6, "k={k}");
        }
        assert!(out[LINES..].iter().all(|&x| x == 0.0));
    }

    /// Feed a steady cosine at the centre frequency of line `k` into
    /// subband `sb` and return the index of the line with the most energy.
    ///
    /// original: a single MDCT frame of a tone depends on its phase and can
    /// peak on a neighbouring line, so energy is summed over several
    /// granules after a warm-up granule.
    fn peak_line(sb: usize, k: usize) -> usize {
        let mut mdct = Mdct::new();
        let mut out = [0.0f32; GRANULE_LINES];
        let mut energy = [0.0f64; GRANULE_LINES];
        for g in 0..9 {
            let input: SubbandGranule = std::array::from_fn(|t| {
                let n = (g * LINES + t) as f64;
                let mut row = [0.0f32; SUBBANDS];
                // MDCT basis frequency for line k: π/18 · (k + 1/2) per sample.
                let mut v = (PI / LINES as f64 * (k as f64 + 0.5) * n + 0.3).cos() as f32;
                // A real odd subband arrives frequency-inverted; pre-invert so
                // the encoder's inversion restores the intended tone.
                if sb % 2 == 1 && t % 2 == 1 {
                    v = -v;
                }
                row[sb] = v;
                row
            });
            mdct.granule(&input, &mut out);
            if g > 0 {
                for (e, &x) in energy.iter_mut().zip(&out) {
                    *e += f64::from(x) * f64::from(x);
                }
            }
        }
        energy
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0
    }

    #[test]
    fn tones_land_on_expected_line() {
        for (sb, k) in [(0, 0), (0, 7), (3, 17), (10, 4), (31, 12), (31, 17)] {
            assert_eq!(peak_line(sb, k), sb * LINES + k, "sb={sb} k={k}");
        }
    }
}
