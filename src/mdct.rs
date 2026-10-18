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
//! Status: long blocks (`block_type` 0) only.

use std::f64::consts::PI;

use crate::filterbank::AnalysisFilterbank;
pub use crate::filterbank::SUBBANDS;
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

/// Alias-reduction coefficients `c_i`, 11172-3 Table 3-B.9.
pub const ALIAS_CI: [f64; 8] = [
    -0.6, -0.535, -0.33, -0.185, -0.095, -0.041, -0.0142, -0.0037,
];

/// Butterfly coefficients from Table 3-B.9:
/// `cs_i = 1/√(1 + c_i²)`, `ca_i = c_i/√(1 + c_i²)`. Computed in f64.
fn alias_coefficients() -> ([f32; 8], [f32; 8]) {
    let norm = |c: f64| (1.0 + c * c).sqrt();
    (
        std::array::from_fn(|i| (1.0 / norm(ALIAS_CI[i])) as f32),
        std::array::from_fn(|i| (ALIAS_CI[i] / norm(ALIAS_CI[i])) as f32),
    )
}

/// Encoder alias reduction for a long-block granule, 11172-3 Annex C
/// ("Aliasing-Butterfly, Encoder") with the layout of Figure 3-A.5: across
/// each of the 31 subband boundaries, butterfly i (0..8) pairs
/// `a = X[18·sb − 1 − i]` with `b = X[18·sb + i]` and computes
/// `a' = a·cs_i + b·ca_i`, `b' = b·cs_i − a·ca_i`.
///
/// This is the inverse rotation of the decoder butterfly (Figure 3-A.6,
/// `a' = a·cs − b·ca`, `b' = b·cs + a·ca`), since `cs² + ca² = 1`.
pub fn alias_reduce(xr: &mut [f32; GRANULE_LINES], cs: &[f32; 8], ca: &[f32; 8]) {
    for sb in 1..SUBBANDS {
        for i in 0..8 {
            let (lo, hi) = (LINES * sb - 1 - i, LINES * sb + i);
            let (a, b) = (xr[lo], xr[hi]);
            xr[lo] = a * cs[i] + b * ca[i];
            xr[hi] = b * cs[i] - a * ca[i];
        }
    }
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

/// PCM samples per granule.
pub const GRANULE_SAMPLES: usize = GRANULE_LINES;

/// The encoder's hybrid filterbank for one channel (long blocks): polyphase
/// analysis (Annex C.1.3), MDCT (§2.4.3.4) and alias reduction (Annex C).
///
/// Through a conforming decoder the PCM comes back at unit gain, delayed by
/// [`HybridFilterbank::DELAY`] samples (checked by `hybrid_round_trip`).
#[derive(Debug, Clone)]
pub struct HybridFilterbank {
    analysis: AnalysisFilterbank,
    mdct: Mdct,
    cs: [f32; 8],
    ca: [f32; 8],
}

impl Default for HybridFilterbank {
    fn default() -> Self {
        Self::new()
    }
}

impl HybridFilterbank {
    /// Encoder plus decoder delay in samples. original: measured with the
    /// reference decoder in `hybrid_round_trip`. It splits as one granule
    /// (576) for the MDCT overlap plus 481 for the analysis and synthesis
    /// polyphase filters together.
    pub const DELAY: usize = 1057;

    pub fn new() -> Self {
        let (cs, ca) = alias_coefficients();
        Self {
            analysis: AnalysisFilterbank::new(),
            mdct: Mdct::new(),
            cs,
            ca,
        }
    }

    /// Turn 576 PCM samples (oldest first) into 576 frequency lines.
    pub fn granule(&mut self, pcm: &[f32; GRANULE_SAMPLES], out: &mut [f32; GRANULE_LINES]) {
        let mut sub: SubbandGranule = [[0.0; SUBBANDS]; LINES];
        for (row, chunk) in sub.iter_mut().zip(pcm.chunks_exact(SUBBANDS)) {
            let input: &[f32; SUBBANDS] = chunk.try_into().expect("chunk of 32");
            self.analysis.process(input, row);
        }
        self.mdct.granule(&sub, out);
        alias_reduce(out, &self.cs, &self.ca);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refdec::{HybridSynthesis, Imdct};

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
            let rec = imdct.granule(&lines.map(f64::from));
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

    #[test]
    fn alias_butterflies_are_inverse_rotations() {
        let (cs, ca) = alias_coefficients();
        for i in 0..8 {
            let r = f64::from(cs[i]).powi(2) + f64::from(ca[i]).powi(2);
            assert!((r - 1.0).abs() < 1e-6, "i={i}");
        }
        // Encoder then reference-decoder butterflies restore the input.
        let mut seed = 3;
        let orig: [f32; GRANULE_LINES] = std::array::from_fn(|_| noise(&mut seed));
        let mut xr = orig;
        alias_reduce(&mut xr, &cs, &ca);
        assert_ne!(xr, orig);
        let mut back = xr.map(f64::from);
        crate::refdec::alias_butterflies(&mut back);
        for (b, o) in back.iter().zip(&orig) {
            assert!((b - f64::from(*o)).abs() < 1e-6);
        }
        // Lines more than 8 away from any boundary are untouched.
        for sb in 0..SUBBANDS {
            for k in 8..10 {
                assert_eq!(xr[sb * LINES + k], orig[sb * LINES + k]);
            }
        }
    }

    /// Encoder hybrid filterbank into the spec's decoder chain: PCM must come
    /// back at unit gain after `DELAY` samples, limited only by the 16-bit
    /// window coefficients (near-perfect reconstruction).
    #[test]
    fn hybrid_round_trip() {
        let granules = 30;
        let mut seed = 11;
        let input: Vec<f32> = (0..granules * GRANULE_SAMPLES)
            .map(|_| 0.5 * noise(&mut seed))
            .collect();
        let mut enc = HybridFilterbank::new();
        let mut dec = HybridSynthesis::new();
        let mut lines = [0.0f32; GRANULE_LINES];
        let mut output = Vec::new();
        for chunk in input.chunks_exact(GRANULE_SAMPLES) {
            enc.granule(chunk.try_into().unwrap(), &mut lines);
            output.extend(dec.granule(&lines));
        }
        let d = HybridFilterbank::DELAY;
        // Skip the start-up transient of the filters.
        let (mut sig, mut err) = (0.0f64, 0.0f64);
        for n in 2 * GRANULE_SAMPLES..input.len() - d {
            let x = f64::from(input[n]);
            sig += x * x;
            err += (output[n + d] - x).powi(2);
        }
        let snr = 10.0 * (sig / err).log10();
        assert!(snr > 80.0, "round-trip SNR {snr:.1} dB");
    }

    #[test]
    fn sine_lands_on_expected_line() {
        // A PCM sine at the centre of line L = 18·sb + k sits at
        // (L + 1/2) · fs/1152 (576 lines spanning 0..fs/2).
        for line in [3usize, 40, 100, 301, 450] {
            let w = (line as f64 + 0.5) * PI / GRANULE_LINES as f64;
            let mut enc = HybridFilterbank::new();
            let mut lines = [0.0f32; GRANULE_LINES];
            let mut energy = [0.0f64; GRANULE_LINES];
            for g in 0..12 {
                let pcm: [f32; GRANULE_SAMPLES] =
                    std::array::from_fn(|i| (w * (g * GRANULE_SAMPLES + i) as f64).sin() as f32);
                enc.granule(&pcm, &mut lines);
                if g >= 3 {
                    for (e, &x) in energy.iter_mut().zip(&lines) {
                        *e += f64::from(x) * f64::from(x);
                    }
                }
            }
            let peak = (0..GRANULE_LINES)
                .max_by(|&a, &b| energy[a].total_cmp(&energy[b]))
                .unwrap();
            assert_eq!(peak, line);
        }
    }
}
