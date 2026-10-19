//! Psychoacoustic model: spectrum, tonality, spreading, masking threshold, perceptual entropy.
//!
//! **Milestone:** M5. See docs/PLAN.md.
//!
//! **Allowed sources:** ISO/IEC 11172-3 Annex D (Model 2 as the starting point); Johnston 1988; Painter & Spanias 2000; Zwicker & Fastl.
//!
//! **Must be tested by:** Threshold of a pure tone vs. noise masker should match published masking curves in shape; PE should spike on attacks.
//!
//! Clean-room reminder: derive from the sources above only. Cite clauses in
//! comments and log every source in PROVENANCE.md (CLEANROOM.md).
//!
//! Implements psychoacoustic model 2 (11172-3 Annex D §3-D.2) with the
//! Layer III adaptations of Annex C.1.5.3 §2.1, long blocks only (short
//! blocks are M6), at the MPEG-1 sample rates. For each granule it returns a
//! signal-to-mask ratio per scalefactor band, which the encoder turns into
//! the outer loop's allowed distortion.

use std::f64::consts::PI;

use crate::quantize::ScalefactorBands;

mod tables {
    //! Annex C Tables 3-C.7a–c and 3-C.8a–c, transcribed from the CD 11172-3
    //! rev 1 text (see PROVENANCE.md). Checked at transcription: partition
    //! numbering is contiguous and bval increasing; in Table 3-C.8 every
    //! band's `bo` is the next band's `bu`, and `w2[i] + w1[i+1] = 1` to
    //! print precision. The 44.1 and 32 kHz conversion tables stop at band 19
    //! in this copy; see `PsyModel::new` for bands 20 and 21.
    #![cfg_attr(rustfmt, rustfmt::skip)]
    /// Table 3-C.7b (44.1 kHz, long blocks): `(FFT lines, minval dB, qthr, norm, bval)` per partition.
    pub const PART_44100: [(u16, f32, f32, f32, f32); 63] = [
        (1, 24.5, 4.532, 0.951, 0.000), (1, 24.5, 4.532, 0.700, 0.431), (1, 24.5, 4.532, 0.681, 0.861),
        (1, 24.5, 0.904, 0.675, 1.292), (1, 24.5, 0.904, 0.667, 1.723), (1, 20.0, 0.090, 0.665, 2.153),
        (1, 20.0, 0.090, 0.664, 2.584), (1, 20.0, 0.029, 0.664, 3.015), (1, 20.0, 0.029, 0.664, 3.445),
        (1, 20.0, 0.029, 0.655, 3.876), (1, 20.0, 0.009, 0.616, 4.279), (1, 20.0, 0.009, 0.597, 4.670),
        (1, 18.0, 0.009, 0.578, 5.057), (1, 18.0, 0.009, 0.541, 5.415), (1, 18.0, 0.009, 0.575, 5.774),
        (2, 12.0, 0.018, 0.856, 6.422), (2, 6.0, 0.018, 0.846, 7.026), (2, 6.0, 0.018, 0.840, 7.609),
        (2, 3.0, 0.018, 0.822, 8.168), (2, 3.0, 0.018, 0.800, 8.710), (2, 3.0, 0.018, 0.753, 9.207),
        (2, 3.0, 0.018, 0.704, 9.662), (2, 0.0, 0.018, 0.674, 10.099), (2, 0.0, 0.018, 0.640, 10.515),
        (2, 0.0, 0.018, 0.609, 10.917), (2, 0.0, 0.018, 0.566, 11.293), (2, 0.0, 0.018, 0.535, 11.652),
        (2, 0.0, 0.018, 0.531, 11.997), (3, 0.0, 0.027, 0.615, 12.394), (3, 0.0, 0.027, 0.686, 12.850),
        (3, 0.0, 0.027, 0.650, 13.277), (3, 0.0, 0.027, 0.611, 13.681), (3, 0.0, 0.027, 0.567, 14.062),
        (3, 0.0, 0.027, 0.520, 14.411), (3, 0.0, 0.027, 0.513, 14.751), (4, 0.0, 0.036, 0.557, 15.119),
        (4, 0.0, 0.036, 0.584, 15.508), (4, 0.0, 0.036, 0.570, 15.883), (5, 0.0, 0.045, 0.579, 16.263),
        (5, 0.0, 0.045, 0.585, 16.654), (5, 0.0, 0.045, 0.548, 17.020), (6, 0.0, 0.054, 0.536, 17.374),
        (6, 0.0, 0.054, 0.550, 17.744), (7, 0.0, 0.063, 0.532, 18.104), (7, 0.0, 0.063, 0.504, 18.447),
        (7, 0.0, 0.063, 0.496, 18.781), (9, 0.0, 0.081, 0.516, 19.130), (9, 0.0, 0.081, 0.527, 19.487),
        (9, 0.0, 0.081, 0.516, 19.838), (10, 0.0, 0.180, 0.497, 20.179), (10, 0.0, 0.180, 0.489, 20.510),
        (11, 0.0, 0.198, 0.502, 20.852), (14, 0.0, 0.400, 0.502, 21.196), (14, 0.0, 0.400, 0.491, 21.531),
        (15, 0.0, 0.429, 0.497, 21.870), (20, 0.0, 1.808, 0.504, 22.214), (20, 0.0, 1.808, 0.504, 22.558),
        (21, 0.0, 1.899, 0.495, 22.898), (27, 0.0, 24.415, 0.486, 23.232), (27, 0.0, 24.415, 0.484, 23.564),
        (36, 0.0, 32.554, 0.483, 23.897), (73, 0.0, 660.124, 0.475, 24.229), (18, 0.0, 162.770, 0.515, 24.542),
    ];
    
    /// Table 3-C.8b (44.1 kHz, long blocks): `(bu, bo, w1, w2)` per scalefactor band.
    pub const CONV_44100: [(u8, u8, f32, f32); 20] = [
        (0, 4, 1.000, 0.056), (4, 7, 0.944, 0.611), (7, 11, 0.389, 0.167), (11, 14, 0.833, 0.722),
        (14, 17, 0.278, 0.139), (17, 18, 0.861, 0.917), (18, 21, 0.083, 0.583), (21, 24, 0.417, 0.250),
        (24, 27, 0.750, 0.805), (27, 30, 0.194, 0.574), (30, 33, 0.426, 0.537), (33, 36, 0.463, 0.819),
        (36, 40, 0.180, 0.100), (40, 43, 0.900, 0.468), (43, 46, 0.532, 0.623), (46, 49, 0.376, 0.450),
        (49, 52, 0.550, 0.552), (52, 55, 0.448, 0.403), (55, 57, 0.597, 0.643), (57, 59, 0.357, 0.722),
    ];
    
    /// Table 3-C.7a (48 kHz, long blocks): `(FFT lines, minval dB, qthr, norm, bval)` per partition.
    pub const PART_48000: [(u16, f32, f32, f32, f32); 62] = [
        (1, 24.5, 4.532, 0.970, 0.000), (1, 24.5, 4.532, 0.755, 0.469), (1, 24.5, 4.532, 0.738, 0.937),
        (1, 24.5, 0.904, 0.730, 1.406), (1, 24.5, 0.904, 0.724, 1.875), (1, 20.0, 0.090, 0.723, 2.344),
        (1, 20.0, 0.090, 0.723, 2.812), (1, 20.0, 0.029, 0.723, 3.281), (1, 20.0, 0.029, 0.718, 3.750),
        (1, 20.0, 0.009, 0.690, 4.199), (1, 20.0, 0.009, 0.660, 4.625), (1, 18.0, 0.009, 0.641, 5.047),
        (1, 18.0, 0.009, 0.600, 5.437), (1, 18.0, 0.009, 0.584, 5.828), (1, 12.0, 0.009, 0.531, 6.187),
        (1, 12.0, 0.009, 0.537, 6.522), (2, 6.0, 0.018, 0.857, 7.174), (2, 6.0, 0.018, 0.858, 7.800),
        (2, 3.0, 0.018, 0.853, 8.402), (2, 3.0, 0.018, 0.824, 8.966), (2, 3.0, 0.018, 0.778, 9.483),
        (2, 3.0, 0.018, 0.740, 9.966), (2, 0.0, 0.018, 0.709, 10.426), (2, 0.0, 0.018, 0.676, 10.866),
        (2, 0.0, 0.018, 0.632, 11.279), (2, 0.0, 0.018, 0.592, 11.669), (2, 0.0, 0.018, 0.553, 12.042),
        (2, 0.0, 0.018, 0.510, 12.386), (2, 0.0, 0.018, 0.513, 12.721), (3, 0.0, 0.027, 0.608, 13.115),
        (3, 0.0, 0.027, 0.673, 13.561), (3, 0.0, 0.027, 0.636, 13.983), (3, 0.0, 0.027, 0.586, 14.371),
        (3, 0.0, 0.027, 0.571, 14.741), (4, 0.0, 0.036, 0.616, 15.140), (4, 0.0, 0.036, 0.640, 15.562),
        (4, 0.0, 0.036, 0.597, 15.962), (4, 0.0, 0.036, 0.538, 16.324), (4, 0.0, 0.036, 0.512, 16.665),
        (5, 0.0, 0.045, 0.528, 17.020), (5, 0.0, 0.045, 0.516, 17.373), (5, 0.0, 0.045, 0.493, 17.708),
        (6, 0.0, 0.054, 0.499, 18.045), (7, 0.0, 0.063, 0.525, 18.398), (7, 0.0, 0.063, 0.541, 18.762),
        (8, 0.0, 0.072, 0.528, 19.120), (8, 0.0, 0.072, 0.510, 19.466), (8, 0.0, 0.072, 0.506, 19.807),
        (10, 0.0, 0.180, 0.525, 20.159), (10, 0.0, 0.180, 0.536, 20.522), (10, 0.0, 0.180, 0.518, 20.873),
        (13, 0.0, 0.372, 0.501, 21.214), (13, 0.0, 0.372, 0.496, 21.553), (14, 0.0, 0.400, 0.497, 21.892),
        (18, 0.0, 1.628, 0.495, 22.231), (18, 0.0, 1.628, 0.494, 22.569), (20, 0.0, 1.808, 0.497, 22.909),
        (25, 0.0, 22.607, 0.494, 23.248), (25, 0.0, 22.607, 0.487, 23.583), (35, 0.0, 31.650, 0.483, 23.915),
        (67, 0.0, 605.867, 0.482, 24.246), (67, 0.0, 605.867, 0.524, 24.576),
    ];
    
    /// Table 3-C.8a (48 kHz, long blocks): `(bu, bo, w1, w2)` per scalefactor band.
    pub const CONV_48000: [(u8, u8, f32, f32); 21] = [
        (0, 4, 1.000, 0.056), (4, 7, 0.944, 0.611), (7, 11, 0.389, 0.167), (11, 14, 0.833, 0.722),
        (14, 17, 0.278, 0.639), (17, 19, 0.361, 0.417), (19, 22, 0.583, 0.083), (22, 24, 0.917, 0.750),
        (24, 27, 0.250, 0.417), (27, 30, 0.583, 0.648), (30, 33, 0.352, 0.611), (33, 36, 0.389, 0.625),
        (36, 40, 0.375, 0.144), (40, 43, 0.856, 0.389), (43, 46, 0.611, 0.160), (46, 49, 0.840, 0.217),
        (49, 52, 0.783, 0.184), (52, 54, 0.816, 0.886), (54, 57, 0.114, 0.313), (57, 59, 0.687, 0.452),
        (59, 60, 0.548, 0.908),
    ];
    
    /// Table 3-C.7c (32 kHz, long blocks): `(FFT lines, minval dB, qthr, norm, bval)` per partition.
    pub const PART_32000: [(u16, f32, f32, f32, f32); 59] = [
        (2, 24.5, 9.064, 0.997, 0.312), (2, 24.5, 9.064, 0.893, 0.937), (2, 24.5, 1.808, 0.881, 1.562),
        (2, 20.0, 0.181, 0.873, 2.187), (2, 20.0, 0.181, 0.872, 2.812), (2, 20.0, 0.057, 0.871, 3.437),
        (2, 20.0, 0.018, 0.860, 4.045), (2, 20.0, 0.018, 0.839, 4.625), (2, 18.0, 0.018, 0.812, 5.173),
        (2, 18.0, 0.018, 0.784, 5.698), (2, 12.0, 0.018, 0.741, 6.184), (2, 12.0, 0.018, 0.697, 6.634),
        (2, 6.0, 0.018, 0.674, 7.070), (2, 6.0, 0.018, 0.651, 7.492), (2, 6.0, 0.018, 0.633, 7.905),
        (2, 3.0, 0.018, 0.611, 8.305), (2, 3.0, 0.018, 0.589, 8.695), (2, 3.0, 0.018, 0.575, 9.064),
        (3, 3.0, 0.027, 0.654, 9.483), (3, 3.0, 0.027, 0.724, 9.966), (3, 0.0, 0.027, 0.701, 10.425),
        (3, 0.0, 0.027, 0.673, 10.866), (3, 0.0, 0.027, 0.631, 11.279), (3, 0.0, 0.027, 0.592, 11.669),
        (3, 0.0, 0.027, 0.553, 12.042), (3, 0.0, 0.027, 0.510, 12.386), (3, 0.0, 0.027, 0.505, 12.721),
        (4, 0.0, 0.036, 0.562, 13.091), (4, 0.0, 0.036, 0.598, 13.488), (4, 0.0, 0.036, 0.589, 13.873),
        (5, 0.0, 0.045, 0.607, 14.268), (5, 0.0, 0.045, 0.620, 14.679), (5, 0.0, 0.045, 0.580, 15.067),
        (5, 0.0, 0.045, 0.532, 15.424), (5, 0.0, 0.045, 0.517, 15.771), (6, 0.0, 0.054, 0.517, 16.120),
        (6, 0.0, 0.054, 0.509, 16.466), (6, 0.0, 0.054, 0.506, 16.807), (8, 0.0, 0.072, 0.522, 17.158),
        (8, 0.0, 0.072, 0.531, 17.518), (8, 0.0, 0.072, 0.519, 17.869), (10, 0.0, 0.090, 0.512, 18.215),
        (10, 0.0, 0.090, 0.509, 18.562), (10, 0.0, 0.090, 0.497, 18.902), (12, 0.0, 0.108, 0.494, 19.239),
        (12, 0.0, 0.108, 0.501, 19.579), (13, 0.0, 0.117, 0.507, 19.925), (14, 0.0, 0.252, 0.502, 20.269),
        (14, 0.0, 0.252, 0.493, 20.606), (16, 0.0, 0.289, 0.497, 20.944), (20, 0.0, 0.572, 0.506, 21.288),
        (20, 0.0, 0.572, 0.510, 21.635), (23, 0.0, 0.658, 0.504, 21.979), (27, 0.0, 2.441, 0.496, 22.319),
        (27, 0.0, 2.441, 0.493, 22.656), (32, 0.0, 2.894, 0.490, 22.993), (37, 0.0, 33.458, 0.483, 23.326),
        (37, 0.0, 33.458, 0.458, 23.656), (12, 0.0, 10.851, 0.500, 23.937),
    ];
    
    /// Table 3-C.8c (32 kHz, long blocks): `(bu, bo, w1, w2)` per scalefactor band.
    pub const CONV_32000: [(u8, u8, f32, f32); 20] = [
        (0, 2, 1.000, 0.528), (2, 4, 0.472, 0.305), (4, 6, 0.694, 0.083), (6, 7, 0.917, 0.861),
        (7, 9, 0.139, 0.639), (9, 11, 0.361, 0.417), (11, 14, 0.583, 0.083), (14, 16, 0.917, 0.750),
        (16, 19, 0.250, 0.870), (19, 22, 0.130, 0.833), (22, 26, 0.167, 0.389), (26, 30, 0.611, 0.478),
        (30, 34, 0.522, 0.033), (34, 37, 0.967, 0.917), (37, 41, 0.083, 0.617), (41, 44, 0.383, 0.995),
        (44, 48, 0.005, 0.274), (48, 51, 0.726, 0.480), (51, 54, 0.519, 0.261), (54, 56, 0.739, 0.884),
    ];
    
}

/// A Table 3-C.7 row: `(FFT lines, minval dB, qthr, norm, bval)`.
type PartitionRow = (u16, f32, f32, f32, f32);
/// A Table 3-C.8 row: `(bu, bo, w1, w2)`.
type ConversionRow = (u8, u8, f32, f32);

/// Long FFT length (Annex D step 2).
pub const LONG_FFT: usize = 1024;
/// Short FFT length used for the Layer III unpredictability (Annex C.1.5.3
/// §2.1: shift 192, FFT 256).
pub const SHORT_FFT: usize = 256;

/// Where the analysis windows start, relative to the first PCM sample of the
/// granule being coded.
///
/// Annex D input 2: the window must be "centered in the time-window of
/// application". original derivation: the MDCT that codes granule g covers
/// subband samples 18(g−1) .. 18g+17, centred on subband time 18g − ½. A
/// subband sample produced after PCM sample 32t+31 is centred on PCM time
/// 32t + 31 − 240.5 (half the 481-sample polyphase analysis+synthesis delay
/// measured in `mdct::HybridFilterbank::DELAY`), so the granule's MDCT is
/// centred on PCM time 576g − 225.5. The long window (1024) starts 512
/// earlier. The Layer III unpredictability uses "the second short block out
/// of three", predicted from the two before it; we centre that block on the
/// same point, so the three short windows (256) are centred 384, 192 and 0
/// samples before it.
pub const LONG_WINDOW_START: isize = -738;
/// Start offsets of the three short windows (oldest first), as above.
pub const SHORT_WINDOW_STARTS: [isize; 3] = [-738, -546, -354];
/// PCM history the model needs before each granule's first sample.
pub const HISTORY: usize = 738;

/// Constants of Annex C.1.5.3 §2.1 (Layer III adaptation of model 2).
const CONV1: f32 = -0.299;
const CONV2: f32 = -0.43;
/// Tone masking noise and noise masking tone, dB, for all partitions.
const TMN_DB: f32 = 29.0;
const NMT_DB: f32 = 6.0;
/// Pre-echo control: the threshold may not exceed `RPELEV` times the
/// previous granule's or `RPELEV2` times the one before that.
const RPELEV: f32 = 2.0;
const RPELEV2: f32 = 16.0;
/// Scale applied to Table 3-C.7's threshold in quiet `qthr`.
///
/// Tuned (eval runs `m5a-psy-over-128` and `m5b-q*-over-128`, dev corpus,
/// 128 kbps, mean ViSQOL): ×1 4.29, ×0.1 4.42, ×0.01 4.48, ×0 4.46. The
/// threshold in quiet assumes full scale plays at 96 dB SPL; quietly
/// mastered material is played louder, and at ×1 the model declared real
/// 12–20 kHz content inaudible, so the outer loop removed it (the same
/// effect that sank M4's absolute-threshold loop). ×0.01 assumes up to
/// 20 dB more playback gain. A tuning constant: re-check it against the
/// holdout corpus at a listening gate.
const QTHR_SCALE: f32 = 0.01;

/// Unpredictability used above the lines it is computed for.
const C_DEFAULT: f32 = 0.4;
/// The unpredictability comes from the long FFT for lines 0..6 and from the
/// short FFT for lines 6..206 (Annex C.1.5.3 §2.1).
const C_LONG_LINES: usize = 6;
const C_SHORT_LINES: usize = 206;

/// Layer III spreading function, Annex D §3-D.2 with the Annex C.1.5.3 §2.1
/// replacement of the Bark-distance scaling, for masker Bark `i` spreading
/// into Bark `j`.
///
/// The CD 11172-3 text prints the replacement as "if j ≥ i, tmpy = 3.0(j−i),
/// else tmpy = 1.5(j−i)". Read literally (replacing `tmpy`) the function
/// would grow with distance, so it must replace the distance scaling
/// `tmpx = 1.05(j−i)`. The direction is settled by the spec's own data:
/// only `tmpx = 1.5(j−i)` for j ≥ i and `3.0(j−i)` for j < i reproduces the
/// `norm` column of Table 3-C.7 (to 0.002 at all three rates; checked in
/// `norm_column_matches_spreading_function`). Values below 10^-6 are zero.
fn spreading(i: f64, j: f64) -> f64 {
    let tmpx = if j >= i { 1.5 * (j - i) } else { 3.0 * (j - i) };
    let x = 8.0 * ((tmpx - 0.5).powi(2) - 2.0 * (tmpx - 0.5)).min(0.0);
    let tmpy = 15.811389 + 7.5 * (tmpx + 0.474) - 17.5 * (1.0 + (tmpx + 0.474).powi(2)).sqrt();
    if tmpy < -100.0 {
        return 0.0;
    }
    let v = 10f64.powf((x + tmpy) / 10.0);
    if v < 1e-6 { 0.0 } else { v }
}

/// In-place iterative radix-2 complex FFT (original implementation of the
/// standard Cooley–Tukey algorithm). Twiddles computed in f64.
#[derive(Debug, Clone)]
struct Fft {
    n: usize,
    twiddle: Vec<(f32, f32)>,
}

impl Fft {
    fn new(n: usize) -> Self {
        assert!(n.is_power_of_two());
        let twiddle = (0..n / 2)
            .map(|k| {
                let a = -2.0 * PI * k as f64 / n as f64;
                (a.cos() as f32, a.sin() as f32)
            })
            .collect();
        Self { n, twiddle }
    }

    fn run(&self, re: &mut [f32], im: &mut [f32]) {
        let n = self.n;
        let bits = n.trailing_zeros();
        for i in 0..n {
            let j = i.reverse_bits() >> (usize::BITS - bits);
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let step = n / len;
            for start in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let (wr, wi) = self.twiddle[k * step];
                    let (a, b) = (start + k, start + k + len / 2);
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len *= 2;
        }
    }
}

/// Hann window of Annex D step 2: `0.5 − 0.5·cos(2π(i − ½)/N)`, i = 1..N.
fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| (0.5 - 0.5 * (2.0 * PI * (i as f64 + 0.5) / n as f64).cos()) as f32)
        .collect()
}

/// Magnitude and phase of a windowed real block, bins 0..=N/2.
///
/// original: input is scaled to 16-bit integer units and the transform by
/// 1/N. In these units a ±½ LSB sine has about 0.023 energy in its main lobe,
/// which matches the scale of Table 3-C.7's `qthr` column (0.018–0.027 per
/// partition where hearing is most sensitive), so `qthr` and the "+1" in the
/// perceptual entropy can be used as printed.
fn spectrum(fft: &Fft, window: &[f32], samples: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let n = fft.n;
    let scale = 32768.0 / n as f32;
    let mut re: Vec<f32> = samples
        .iter()
        .zip(window)
        .map(|(s, w)| s * w * scale)
        .collect();
    let mut im = vec![0.0f32; n];
    fft.run(&mut re, &mut im);
    let bins = n / 2 + 1;
    let r = (0..bins).map(|k| re[k].hypot(im[k])).collect();
    let f = (0..bins).map(|k| im[k].atan2(re[k])).collect();
    (r, f)
}

/// Unpredictability of one bin from its magnitude/phase and the two
/// previous blocks' (Annex D steps 3–4).
fn unpredictability(r: f32, f: f32, r1: f32, f1: f32, r2: f32, f2: f32) -> f32 {
    let rp = 2.0 * r1 - r2;
    let fp = 2.0 * f1 - f2;
    let den = r + rp.abs();
    if den == 0.0 {
        return 0.0;
    }
    let dx = r * f.cos() - rp * fp.cos();
    let dy = r * f.sin() - rp * fp.sin();
    dx.hypot(dy) / den
}

/// Per-granule output of the model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PsyResult {
    /// Masking threshold over signal energy, per scalefactor band 0..=21.
    pub ratio: [f32; 22],
    /// Perceptual entropy (Annex C.1.5.3 §2.1), natural log.
    pub pe: f32,
}

/// Psychoacoustic model 2 state for one channel.
#[derive(Debug, Clone)]
pub struct PsyModel {
    /// Per partition: (first FFT line, number of lines, minval, qthr, norm).
    parts: Vec<(usize, usize, f32, f32, f32)>,
    /// `spread[b * n + bb]`: spreading from partition bb into partition b.
    spread: Vec<f32>,
    /// Per scalefactor band 0..=21: (partition, weight) pairs, or empty
    /// when no partition maps there.
    band_parts: Vec<Vec<(usize, f32)>>,
    fft_long: Fft,
    fft_short: Fft,
    hann_long: Vec<f32>,
    hann_short: Vec<f32>,
    /// Long-FFT magnitude and phase of the two previous granules, for the
    /// unpredictability of lines 0..6.
    prev: [(Vec<f32>, Vec<f32>); 2],
    /// Partition thresholds `nb` of the two previous granules (pre-echo
    /// control).
    nb_prev: [Vec<f32>; 2],
}

impl PsyModel {
    /// Model for an MPEG-1 sample rate; `None` for LSF rates (their tables
    /// are in 13818-3 Annex D; M8).
    pub fn new(sample_rate: u32, bands: &ScalefactorBands) -> Option<Self> {
        let (part, conv): (&[PartitionRow], &[ConversionRow]) = match sample_rate {
            44_100 => (&tables::PART_44100, &tables::CONV_44100),
            48_000 => (&tables::PART_48000, &tables::CONV_48000),
            32_000 => (&tables::PART_32000, &tables::CONV_32000),
            _ => return None,
        };
        let mut lo = 0;
        let parts: Vec<_> = part
            .iter()
            .map(|&(lines, minval, qthr, norm, _)| {
                let p = (lo, usize::from(lines), minval, qthr, norm);
                lo += usize::from(lines);
                p
            })
            .collect();
        let n = parts.len();
        let bval: Vec<f64> = part.iter().map(|p| f64::from(p.4)).collect();
        let mut spread = vec![0.0f32; n * n];
        for b in 0..n {
            for bb in 0..n {
                spread[b * n + bb] = spreading(bval[bb], bval[b]) as f32;
            }
        }

        // Partition-to-band weights from Table 3-C.8: w1 for the first
        // partition `bu`, 1 for those strictly between, w2 for the last `bo`.
        let mut band_parts: Vec<Vec<(usize, f32)>> = vec![Vec::new(); 22];
        for (sb, &(bu, bo, w1, w2)) in conv.iter().enumerate() {
            let (bu, bo) = (usize::from(bu), usize::from(bo));
            band_parts[sb].push((bu, w1));
            band_parts[sb].extend((bu + 1..bo).map(|b| (b, 1.0)));
            band_parts[sb].push((bo, w2));
        }
        // original: bands the table does not cover (band 20 at 44.1 and
        // 32 kHz in our copy, band 21 everywhere) take the rest of the last
        // listed partition and every later partition, each assigned to the
        // band containing its centre frequency (FFT line k is at
        // k · fs/1024, MDCT line l at (l + ½) · fs/1152).
        let (last_bo, last_w2) = conv.last().map(|c| (usize::from(c.1), c.3)).unwrap();
        let first_free = conv.len();
        let bounds = bands.region_bounds();
        let band_of = |b: usize| -> usize {
            let centre = parts[b].0 as f64 + (parts[b].1 as f64 - 1.0) / 2.0;
            let mdct_line = centre * 1152.0 / 1024.0;
            let band = bounds
                .windows(2)
                .position(|w| mdct_line < w[1] as f64)
                .unwrap_or(21);
            band.clamp(first_free, 21)
        };
        band_parts[band_of(last_bo)].push((last_bo, 1.0 - last_w2));
        for b in last_bo + 1..n {
            band_parts[band_of(b)].push((b, 1.0));
        }

        Some(Self {
            parts,
            spread,
            band_parts,
            fft_long: Fft::new(LONG_FFT),
            fft_short: Fft::new(SHORT_FFT),
            hann_long: hann(LONG_FFT),
            hann_short: hann(SHORT_FFT),
            prev: [
                (vec![0.0; LONG_FFT / 2 + 1], vec![0.0; LONG_FFT / 2 + 1]),
                (vec![0.0; LONG_FFT / 2 + 1], vec![0.0; LONG_FFT / 2 + 1]),
            ],
            nb_prev: [vec![0.0; n], vec![0.0; n]],
        })
    }

    /// Analyse one granule. `long` is the 1024-sample window starting at
    /// [`LONG_WINDOW_START`]; `short` the three 256-sample windows at
    /// [`SHORT_WINDOW_STARTS`], oldest first. Samples in [-1, 1].
    pub fn analyze(&mut self, long: &[f32], short: [&[f32]; 3]) -> PsyResult {
        assert_eq!(long.len(), LONG_FFT);
        let (r, f) = spectrum(&self.fft_long, &self.hann_long, long);

        // Unpredictability (Annex C.1.5.3 §2.1): long FFT below line 6, the
        // short FFT's middle block (predicted from the two before it) for
        // lines 6..206, 0.4 above. Long line w maps to short line
        // (w + 2) / 4, grouping lines 6–9, 10–13, ... as the Annex's
        // "w = 6, 10, 14, ..." does.
        let mut c = vec![C_DEFAULT; r.len()];
        let [(r1, f1), (r2, f2)] = &self.prev;
        for w in 0..C_LONG_LINES {
            c[w] = unpredictability(r[w], f[w], r1[w], f1[w], r2[w], f2[w]);
        }
        let s: Vec<(Vec<f32>, Vec<f32>)> = short
            .iter()
            .map(|blk| {
                assert_eq!(blk.len(), SHORT_FFT);
                spectrum(&self.fft_short, &self.hann_short, blk)
            })
            .collect();
        for (w, cw) in c
            .iter_mut()
            .enumerate()
            .take(C_SHORT_LINES)
            .skip(C_LONG_LINES)
        {
            let k = (w + 2) / 4;
            *cw = unpredictability(
                s[2].0[k], s[2].1[k], s[1].0[k], s[1].1[k], s[0].0[k], s[0].1[k],
            );
        }

        self.prev.swap(0, 1);
        self.prev[0] = (r.clone(), f);

        // Energy and weighted unpredictability per partition (step 5).
        let n = self.parts.len();
        let mut eb = vec![0.0f32; n];
        let mut cb = vec![0.0f32; n];
        for (b, &(lo, len, ..)) in self.parts.iter().enumerate() {
            for w in lo..(lo + len).min(r.len()) {
                let e = r[w] * r[w];
                eb[b] += e;
                cb[b] += e * c[w];
            }
        }

        // Spreading, tonality, required SNR, threshold, pre-echo control
        // (steps 6–10, 13 with the Layer III constants).
        let mut thr = vec![0.0f32; n];
        let mut nb = vec![0.0f32; n];
        for b in 0..n {
            let row = &self.spread[b * n..(b + 1) * n];
            let ecb: f32 = row.iter().zip(&eb).map(|(s, e)| s * e).sum();
            let ct: f32 = row.iter().zip(&cb).map(|(s, c)| s * c).sum();
            let (_, _, minval, qthr, norm) = self.parts[b];
            let tb = if ecb > 0.0 {
                (CONV1 + CONV2 * (ct / ecb).max(1e-30).ln()).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let snr = minval.max(TMN_DB * tb + NMT_DB * (1.0 - tb));
            nb[b] = ecb * norm * 10f32.powf(-snr / 10.0);
            // original: pre-echo control limits how fast the threshold may
            // rise. The Annex C flow chart prints
            // "thr = MAX(qthr, nbb, rpelev·nbb_l, rpelev2·nbb_ll)", but for a
            // steady signal that would sit 12 dB (×16) above nbb at all
            // times; only the minimum of the three, floored by qthr, matches
            // the stated purpose.
            let limited = nb[b]
                .min(RPELEV * self.nb_prev[0][b])
                .min(RPELEV2 * self.nb_prev[1][b]);
            thr[b] = (qthr * QTHR_SCALE).max(limited);
        }
        self.nb_prev.swap(0, 1);
        self.nb_prev[0] = nb;

        // Perceptual entropy (Annex C.1.5.3 §2.1).
        let pe: f32 = self
            .parts
            .iter()
            .zip(thr.iter().zip(&eb))
            .map(|(&(_, len, ..), (&t, &e))| -(len as f32) * (t / (e + 1.0)).ln())
            .sum();

        // Convert to scalefactor bands (Table 3-C.8) and form ratios.
        let mut ratio = [0.0f32; 22];
        for (sb, list) in self.band_parts.iter().enumerate() {
            let (en, th) = list.iter().fold((0.0f32, 0.0f32), |(en, th), &(b, w)| {
                (en + w * eb[b], th + w * thr[b])
            });
            ratio[sb] = if list.is_empty() {
                ratio[sb.saturating_sub(1)]
            } else if en > 0.0 {
                th / en
            } else {
                0.0
            };
        }
        PsyResult { ratio, pe }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mdct::{GRANULE_LINES, GRANULE_SAMPLES, HybridFilterbank};

    #[test]
    fn norm_column_matches_spreading_function() {
        // Table 3-C.7's norm(b) = 1 / Σ_bb sprdngf(bval_bb, bval_b) (Annex D
        // step 6). This pins down the Layer III spreading function.
        for rate in [44_100, 48_000, 32_000] {
            let part: &[PartitionRow] = match rate {
                44_100 => &tables::PART_44100,
                48_000 => &tables::PART_48000,
                _ => &tables::PART_32000,
            };
            for b in 0..part.len() {
                let sum: f64 = part
                    .iter()
                    .map(|p| spreading(f64::from(p.4), f64::from(part[b].4)))
                    .sum();
                let norm = 1.0 / sum;
                assert!(
                    (norm - f64::from(part[b].3)).abs() < 0.003,
                    "{rate} partition {b}: {norm} vs {}",
                    part[b].3
                );
            }
        }
    }

    #[test]
    fn masking_spreads_further_upward() {
        assert!((spreading(10.0, 10.0) - 1.0).abs() < 0.01);
        for d in [1.0, 2.0, 3.0] {
            assert!(
                spreading(10.0, 10.0 + d) > spreading(10.0, 10.0 - d),
                "distance {d}"
            );
        }
    }

    #[test]
    fn fft_matches_direct_dft() {
        let n = 16;
        let fft = Fft::new(n);
        let x: Vec<f32> = (0..n).map(|i| ((i * 7 % 5) as f32 - 2.0) * 0.3).collect();
        let (mut re, mut im) = (x.clone(), vec![0.0; n]);
        fft.run(&mut re, &mut im);
        for k in 0..n {
            let (mut dr, mut di) = (0.0f64, 0.0f64);
            for (i, &v) in x.iter().enumerate() {
                let a = -2.0 * PI * (k * i) as f64 / n as f64;
                dr += f64::from(v) * a.cos();
                di += f64::from(v) * a.sin();
            }
            assert!(
                (f64::from(re[k]) - dr).abs() < 1e-5 && (f64::from(im[k]) - di).abs() < 1e-5,
                "bin {k}"
            );
        }
    }

    /// Run the model over a signal granule by granule, as the encoder does.
    fn run(signal: &dyn Fn(isize) -> f32, granules: usize) -> Vec<PsyResult> {
        let bands = ScalefactorBands::long(44_100).unwrap();
        let mut m = PsyModel::new(44_100, &bands).unwrap();
        (0..granules)
            .map(|g| {
                let base = (g * GRANULE_SAMPLES) as isize;
                let win = |start: isize, len: usize| -> Vec<f32> {
                    (0..len)
                        .map(|i| signal(base + start + i as isize))
                        .collect()
                };
                let long = win(LONG_WINDOW_START, LONG_FFT);
                let s: Vec<Vec<f32>> = SHORT_WINDOW_STARTS
                    .iter()
                    .map(|&st| win(st, SHORT_FFT))
                    .collect();
                m.analyze(&long, [&s[0], &s[1], &s[2]])
            })
            .collect()
    }

    fn noise(n: isize) -> f32 {
        // original: integer hash of the sample index -> uniform [-1, 1).
        let mut x = (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        x ^= x >> 29;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 32;
        (x >> 40) as f32 / f32::from(1u16 << 15) / 256.0 * 2.0 - 1.0
    }

    #[test]
    fn tone_needs_more_snr_than_noise() {
        // Band 15 at 44.1 kHz spans MDCT lines 134..162, about 5.1–6.2 kHz.
        let f = 5_600.0 / 44_100.0;
        let tone = run(
            &|n| 0.3 * (2.0 * std::f32::consts::PI * f * n as f32).sin(),
            8,
        );
        let white = run(&|n| 0.3 * noise(n), 8);
        let db = |r: f32| 10.0 * r.log10();
        let (t, w) = (db(tone[7].ratio[15]), db(white[7].ratio[15]));
        // Tone masking noise is 29 dB, noise masking tone 6 dB (minval is 0
        // in this region), so the allowed noise for the tone is far lower.
        assert!(t < -20.0, "tone ratio {t} dB");
        assert!((-12.0..-3.0).contains(&w), "noise ratio {w} dB");
    }

    #[test]
    fn pe_spikes_on_attack_and_aligns_with_mdct() {
        // Quiet noise, then a burst starting mid-way through granule 6.
        let onset = 6 * GRANULE_SAMPLES as isize + 300;
        let sig = |n: isize| {
            if n >= onset {
                0.5 * noise(n)
            } else {
                0.001 * noise(n)
            }
        };
        let res = run(&sig, 10);
        let pe: Vec<f32> = res.iter().map(|r| r.pe).collect();
        let jump = (1..pe.len())
            .max_by(|&a, &b| (pe[a] - pe[a - 1]).total_cmp(&(pe[b] - pe[b - 1])))
            .unwrap();
        assert!(pe[jump] > 2.0 * pe[jump - 1].max(1.0), "PE {pe:?}");

        // The granule where PE first jumps must be the one whose MDCT first
        // sees the burst.
        let mut fb = HybridFilterbank::new();
        let mut xr = [0.0f32; GRANULE_LINES];
        let mut energy = Vec::new();
        for g in 0..10 {
            let pcm: [f32; GRANULE_SAMPLES] =
                std::array::from_fn(|i| sig((g * GRANULE_SAMPLES + i) as isize));
            fb.granule(&pcm, &mut xr);
            energy.push(xr.iter().map(|v| v * v).sum::<f32>());
        }
        let mdct_jump = (1..10)
            .max_by(|&a, &b| {
                (energy[a] / energy[a - 1].max(1e-12))
                    .total_cmp(&(energy[b] / energy[b - 1].max(1e-12)))
            })
            .unwrap();
        assert_eq!(jump, mdct_jump, "PE {pe:?}, MDCT energy {energy:?}");
    }
}
