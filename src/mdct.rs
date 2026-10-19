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
//! Status: all four block types (normal, start, short, stop); no mixed
//! blocks.

use std::f64::consts::PI;

use crate::filterbank::AnalysisFilterbank;
pub use crate::filterbank::SUBBANDS;
/// Subband samples per granule, and MDCT lines per subband (long blocks).
pub const LINES: usize = 18;
/// Frequency lines per granule.
pub const GRANULE_LINES: usize = SUBBANDS * LINES;
/// Long-block MDCT length: 18 new samples plus 18 overlapped.
const N: usize = 2 * LINES;
/// Short-block MDCT length (three per granule).
const NS: usize = 12;
/// Lines per short window per subband, and per short window in total.
pub const SHORT_LINES: usize = NS / 2;
pub const SHORT_WINDOW_LINES: usize = SUBBANDS * SHORT_LINES;

/// Window/transform type of a granule (`block_type`, 11172-3 §2.4.2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Block {
    #[default]
    Normal,
    /// Long-to-short transition.
    Start,
    /// Three short transforms. The 576 outputs are three 192-line spectra,
    /// window-major: line `6·sb + m` of window `w` at `w·192 + 6·sb + m`.
    Short,
    /// Short-to-long transition.
    Stop,
}

/// One granule of polyphase output, time-major: `[time][subband]`.
pub type SubbandGranule = [[f32; SUBBANDS]; LINES];

/// Windows for the long transforms of each block type (11172-3 §2.4.3.4
/// "Windowing"; the encoder uses the decoder's windows so the TDAC
/// conditions hold across transitions):
/// - normal (0): `sin(π/36 (i + ½))`, which satisfies the Princen–Bradley
///   condition `w_i² + w_{i+18}² = 1`;
/// - start (1): long rise for 0..18, 1 for 18..24, short fall
///   `sin(π/12 (i − 18 + ½))` for 24..30, 0 for 30..36;
/// - stop (3): 0 for 0..6, short rise `sin(π/12 (i − 6 + ½))` for 6..12,
///   1 for 12..18, long fall for 18..36.
fn block_window(block: Block) -> [f32; N] {
    let long = |i: usize| (PI / 36.0 * (i as f64 + 0.5)).sin();
    let short = |i: usize| (PI / 12.0 * (i as f64 + 0.5)).sin();
    std::array::from_fn(|i| {
        (match block {
            Block::Normal | Block::Short => long(i),
            Block::Start => match i {
                0..18 => long(i),
                18..24 => 1.0,
                24..30 => short(i - 18),
                _ => 0.0,
            },
            Block::Stop => match i {
                0..6 => 0.0,
                6..12 => short(i - 6),
                12..18 => 1.0,
                _ => long(i),
            },
        }) as f32
    })
}

/// Short window, 11172-3 §2.4.3.4 "Windowing" d: `sin(π/12 (i + ½))`.
fn short_window() -> [f32; NS] {
    std::array::from_fn(|i| (PI / 12.0 * (i as f64 + 0.5)).sin() as f32)
}

/// Short MDCT matrix for n = 12: the same kernel and 4/n scaling as
/// [`mdct_matrix`].
fn short_matrix() -> [[f32; NS]; SHORT_LINES] {
    let n = NS as f64;
    std::array::from_fn(|k| {
        std::array::from_fn(|i| {
            let arg = PI / (2.0 * n) * (2.0 * i as f64 + 1.0 + n / 2.0) * (2.0 * k as f64 + 1.0);
            (4.0 / n * arg.cos()) as f32
        })
    })
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
    /// Long windows for normal, start and stop blocks.
    windows: [[f32; N]; 3],
    short_window: [f32; NS],
    matrix: Box<[[f32; N]; LINES]>,
    short_matrix: [[f32; NS]; SHORT_LINES],
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
            windows: [Block::Normal, Block::Start, Block::Stop].map(block_window),
            short_window: short_window(),
            matrix: mdct_matrix(),
            short_matrix: short_matrix(),
        }
    }

    /// Transform one granule of subband samples into 576 frequency lines.
    /// Long blocks: line `18 · sb + k` (11172-3 §2.4.3.4, "each 18 into one
    /// transform block"). Short blocks: see [`Block::Short`].
    ///
    /// The lines describe the 36-sample block ending with this granule, so a
    /// decoder reproduces the subband samples one granule (18 samples per
    /// subband) late.
    pub fn granule(
        &mut self,
        input: &SubbandGranule,
        block: Block,
        out: &mut [f32; GRANULE_LINES],
    ) {
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

            let x = |i: usize| {
                if i < LINES {
                    self.prev[sb][i]
                } else {
                    cur[i - LINES]
                }
            };
            if block == Block::Short {
                // §2.4.3.4: the three short transforms cover samples 6..18,
                // 12..24 and 18..30 of the 36-sample block (6 zeros at each
                // end in the decoder's overlap).
                for w in 0..3 {
                    let z: [f32; NS] =
                        std::array::from_fn(|i| x(6 + 6 * w + i) * self.short_window[i]);
                    let at = w * SHORT_WINDOW_LINES + sb * SHORT_LINES;
                    for (line, row) in out[at..at + SHORT_LINES].iter_mut().zip(&self.short_matrix)
                    {
                        *line = row.iter().zip(&z).map(|(c, v)| c * v).sum();
                    }
                }
            } else {
                let window = &self.windows[match block {
                    Block::Start => 1,
                    Block::Stop => 2,
                    _ => 0,
                }];
                let z: [f32; N] = std::array::from_fn(|i| x(i) * window[i]);
                let lines = &mut out[sb * LINES..(sb + 1) * LINES];
                for (line, row) in lines.iter_mut().zip(self.matrix.iter()) {
                    *line = row.iter().zip(&z).map(|(c, v)| c * v).sum();
                }
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
    ///
    /// Alias reduction is applied to long-type blocks only: it acts across
    /// subband boundaries of 18-line long spectra, which short blocks do not
    /// have. original reading of §2.4.3.4 (the Committee Draft does not state
    /// the short-block case); confirmed against ffmpeg and minimp3 in
    /// `tests/decode.rs`, where the other choice wrecks short-block SNR.
    pub fn granule(
        &mut self,
        pcm: &[f32; GRANULE_SAMPLES],
        block: Block,
        out: &mut [f32; GRANULE_LINES],
    ) {
        let mut sub: SubbandGranule = [[0.0; SUBBANDS]; LINES];
        for (row, chunk) in sub.iter_mut().zip(pcm.chunks_exact(SUBBANDS)) {
            let input: &[f32; SUBBANDS] = chunk.try_into().expect("chunk of 32");
            self.analysis.process(input, row);
        }
        self.mdct.granule(&sub, block, out);
        if block != Block::Short {
            alias_reduce(out, &self.cs, &self.ca);
        }
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
        let w = block_window(Block::Normal);
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
            mdct.granule(&input, Block::Normal, &mut lines);
            let rec = imdct.granule(&lines.map(f64::from), Block::Normal);
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
        Mdct::new().granule(&input, Block::Normal, &mut out);
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
            mdct.granule(&input, Block::Normal, &mut out);
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
            enc.granule(chunk.try_into().unwrap(), Block::Normal, &mut lines);
            output.extend(dec.granule(&lines, Block::Normal));
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
                enc.granule(&pcm, Block::Normal, &mut lines);
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

    /// Every valid sequence of block types (Annex C.1.5.3 state machine:
    /// normal → start → short… → stop → normal, and stop → start).
    const SEQUENCE: [Block; 14] = [
        Block::Normal,
        Block::Start,
        Block::Short,
        Block::Stop,
        Block::Normal,
        Block::Start,
        Block::Short,
        Block::Short,
        Block::Short,
        Block::Stop,
        Block::Start,
        Block::Short,
        Block::Stop,
        Block::Normal,
    ];

    #[test]
    fn tdac_reconstruction_across_block_switching() {
        let mut mdct = Mdct::new();
        let mut imdct = Imdct::new();
        let mut seed = 21;
        let mut lines = [0.0f32; GRANULE_LINES];
        let mut prev: Option<SubbandGranule> = None;
        let mut max_err = 0.0f32;
        for &block in &SEQUENCE {
            let input: SubbandGranule =
                std::array::from_fn(|_| std::array::from_fn(|_| noise(&mut seed)));
            mdct.granule(&input, block, &mut lines);
            let rec = imdct.granule(&lines.map(f64::from), block);
            if let Some(expect) = prev {
                for t in 0..LINES {
                    for sb in 0..SUBBANDS {
                        max_err = max_err.max((rec[t][sb] - expect[t][sb]).abs());
                    }
                }
            }
            prev = Some(input);
        }
        assert!(max_err < 1e-5, "max reconstruction error {max_err}");
    }

    #[test]
    fn hybrid_round_trip_with_block_switching() {
        let mut seed = 31;
        let mut enc = HybridFilterbank::new();
        let mut dec = HybridSynthesis::new();
        let mut lines = [0.0f32; GRANULE_LINES];
        let mut input = Vec::new();
        let mut output = Vec::new();
        for _ in 0..2 {
            for &block in &SEQUENCE {
                let pcm: [f32; GRANULE_SAMPLES] = std::array::from_fn(|_| 0.5 * noise(&mut seed));
                enc.granule(&pcm, block, &mut lines);
                output.extend(dec.granule(&lines, block));
                input.extend_from_slice(&pcm);
            }
        }
        let d = HybridFilterbank::DELAY;
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
    fn short_block_tone_lands_on_expected_line() {
        // A subband-domain tone at the centre of short line m of subband sb
        // (π/6 · (m + ½) per sample) peaks there in all three windows.
        for (sb, m) in [(0usize, 0usize), (2, 3), (9, 5), (31, 1)] {
            let mut mdct = Mdct::new();
            let mut out = [0.0f32; GRANULE_LINES];
            for g in 0..3 {
                let input: SubbandGranule = std::array::from_fn(|t| {
                    let n = (g * LINES + t) as f64;
                    let mut v = (PI / 6.0 * (m as f64 + 0.5) * n + 0.2).cos() as f32;
                    if sb % 2 == 1 && t % 2 == 1 {
                        v = -v;
                    }
                    let mut row = [0.0f32; SUBBANDS];
                    row[sb] = v;
                    row
                });
                mdct.granule(&input, Block::Short, &mut out);
            }
            let window_energy = |w: usize| -> Vec<f32> {
                (0..SHORT_WINDOW_LINES)
                    .map(|l| out[w * SHORT_WINDOW_LINES + l].powi(2))
                    .collect()
            };
            let total: Vec<f32> = (0..SHORT_WINDOW_LINES)
                .map(|l| (0..3).map(|w| window_energy(w)[l]).sum())
                .collect();
            let peak = (0..SHORT_WINDOW_LINES)
                .max_by(|&a, &b| total[a].total_cmp(&total[b]))
                .unwrap();
            assert_eq!(peak, sb * SHORT_LINES + m, "sb {sb} m {m}");
        }
    }
}
