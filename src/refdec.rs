//! Test-only reference decoder for the hybrid filterbank: the inverse of
//! everything between PCM and the 576 frequency lines of a granule.
//!
//! Written directly from the decoder side of 11172-3 (CD 11172-3 rev 1 text,
//! see PROVENANCE.md), independently of the encoder code it checks, in f64:
//! - §2.4.3.4 "Synthesis filterbank" and Figure 3-A.6: decoder alias butterflies;
//! - §2.4.3.4 "IMDCT" and "Windowing" a): unscaled IMDCT, block_type 0 window;
//! - §2.4.3.4: overlap-add and frequency inversion of odd subbands;
//! - §2.4.3.2 / Figure 3-A.2: polyphase synthesis with `N_ik` and Table 3-B.3.

use std::f64::consts::PI;

use crate::filterbank::WINDOW_Q21;
use crate::mdct::{ALIAS_CI, GRANULE_LINES, LINES, SUBBANDS, SubbandGranule};

/// Decoder alias-reduction butterflies, Figure 3-A.6: for each subband
/// boundary and i = 0..7, with `a = X[18·sb − 1 − i]` and `b = X[18·sb + i]`,
/// `a' = a·cs_i − b·ca_i` and `b' = b·cs_i + a·ca_i`
/// (Table 3-B.9: `cs_i = 1/√(1 + c_i²)`, `ca_i = c_i/√(1 + c_i²)`).
pub fn alias_butterflies(xr: &mut [f64; GRANULE_LINES]) {
    for sb in 1..SUBBANDS {
        for (i, &c) in ALIAS_CI.iter().enumerate() {
            let norm = (1.0 + c * c).sqrt();
            let (cs, ca) = (1.0 / norm, c / norm);
            let (lo, hi) = (LINES * sb - 1 - i, LINES * sb + i);
            let (a, b) = (xr[lo], xr[hi]);
            xr[lo] = a * cs - b * ca;
            xr[hi] = b * cs + a * ca;
        }
    }
}

/// Long-block IMDCT, window, overlap-add and frequency inversion.
pub struct Imdct {
    overlap: [[f64; LINES]; SUBBANDS],
}

impl Imdct {
    pub fn new() -> Self {
        Self {
            overlap: [[0.0; LINES]; SUBBANDS],
        }
    }

    pub fn granule(&mut self, lines: &[f64; GRANULE_LINES]) -> SubbandGranule {
        let n = 36.0;
        let mut out = [[0.0f32; SUBBANDS]; LINES];
        for sb in 0..SUBBANDS {
            let x = &lines[sb * LINES..(sb + 1) * LINES];
            // x_i = Σ_k X_k cos(π/(2n) (2i + 1 + n/2)(2k + 1)), then
            // z_i = x_i sin(π/36 (i + 1/2)).
            let z: Vec<f64> = (0..36)
                .map(|i| {
                    let xi: f64 = (0..LINES)
                        .map(|k| {
                            let arg = PI / (2.0 * n)
                                * (2.0 * i as f64 + 1.0 + n / 2.0)
                                * (2.0 * k as f64 + 1.0);
                            x[k] * arg.cos()
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

/// Polyphase synthesis filterbank, Figure 3-A.2.
pub struct Synthesis {
    v: Vec<f64>,
}

impl Synthesis {
    pub fn new() -> Self {
        Self { v: vec![0.0; 1024] }
    }

    /// 32 subband samples in, 32 PCM samples out (oldest first).
    pub fn process(&mut self, s: &[f32; SUBBANDS]) -> [f64; SUBBANDS] {
        // Shifting: V[i] = V[i − 64] for i = 1023 down to 64.
        self.v.copy_within(..1024 - 64, 64);
        // Matrixing: V_i = Σ_k N_ik S_k, N_ik = cos((16 + i)(2k + 1)π/64).
        for i in 0..64 {
            self.v[i] = (0..SUBBANDS)
                .map(|k| ((16 + i) as f64 * (2 * k + 1) as f64 * PI / 64.0).cos() * f64::from(s[k]))
                .sum();
        }
        // Build U, window by D (Table 3-B.3; D_i = WINDOW_Q21[i] / 2^16, see
        // filterbank.rs), and sum: S_j = Σ_{i=0}^{15} W_{j+32i}.
        let mut u = [0.0f64; 512];
        for i in 0..8 {
            for j in 0..32 {
                u[i * 64 + j] = self.v[i * 128 + j];
                u[i * 64 + 32 + j] = self.v[i * 128 + 96 + j];
            }
        }
        std::array::from_fn(|j| {
            (0..16)
                .map(|i| {
                    let idx = j + 32 * i;
                    u[idx] * f64::from(WINDOW_Q21[idx]) / f64::from(1 << 16)
                })
                .sum()
        })
    }
}

/// Full decoder path for one channel: 576 lines in, 576 PCM samples out.
pub struct HybridSynthesis {
    imdct: Imdct,
    synth: Synthesis,
}

impl HybridSynthesis {
    pub fn new() -> Self {
        Self {
            imdct: Imdct::new(),
            synth: Synthesis::new(),
        }
    }

    pub fn granule(&mut self, lines: &[f32; GRANULE_LINES]) -> Vec<f64> {
        let mut xr: [f64; GRANULE_LINES] = std::array::from_fn(|i| f64::from(lines[i]));
        alias_butterflies(&mut xr);
        let sub = self.imdct.granule(&xr);
        sub.iter().flat_map(|row| self.synth.process(row)).collect()
    }
}
