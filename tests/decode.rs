//! End-to-end decoder tests (docs/PLAN.md M1 and M3 exit criteria): encoded
//! streams must decode without warnings in minimp3 and ffmpeg, with the
//! expected number of frames, and the decoded audio must match the input at
//! unit gain after the filterbank delay.
//!
//! The ffmpeg half is skipped (with a note on stderr) when `ffmpeg` is not on
//! PATH. CI installs it.

use std::path::PathBuf;
use std::process::Command;

use mp3enc::mdct::HybridFilterbank;
use mp3enc::{Encoder, EncoderConfig, NoiseShaping, StereoMode};

struct Case {
    sample_rate: u32,
    channels: usize,
    bitrate: u32,
    stereo: StereoMode,
    /// Minimum SNR (dB) of the decoded audio against the input. original:
    /// set a few dB under what this encoder measured when the case was
    /// added (no psychoacoustics yet), to catch regressions.
    min_snr_db: f64,
    noise_shaping: NoiseShaping,
}

const CASES: &[Case] = &[
    // 44.1 and 22.05 kHz exercise the padding bit; 48 and 32 kHz never pad
    // at these bitrates.
    Case {
        sample_rate: 44_100,
        channels: 2,
        bitrate: 128,
        stereo: StereoMode::Joint,
        min_snr_db: 25.0,
        noise_shaping: NoiseShaping::Off,
    },
    Case {
        sample_rate: 44_100,
        channels: 2,
        bitrate: 320,
        stereo: StereoMode::Stereo,
        min_snr_db: 37.0,
        noise_shaping: NoiseShaping::Off,
    },
    Case {
        sample_rate: 44_100,
        channels: 1,
        bitrate: 64,
        stereo: StereoMode::Joint,
        min_snr_db: 25.0,
        noise_shaping: NoiseShaping::Off,
    },
    Case {
        sample_rate: 48_000,
        channels: 2,
        bitrate: 192,
        stereo: StereoMode::Mono,
        min_snr_db: 40.0,
        noise_shaping: NoiseShaping::Off,
    },
    Case {
        sample_rate: 32_000,
        channels: 1,
        bitrate: 40,
        stereo: StereoMode::Joint,
        min_snr_db: 24.0,
        noise_shaping: NoiseShaping::Off,
    },
    // LSF (13818-3) side-info layout and scalefactor bands.
    Case {
        sample_rate: 22_050,
        channels: 2,
        bitrate: 64,
        stereo: StereoMode::Joint,
        min_snr_db: 24.0,
        noise_shaping: NoiseShaping::Off,
    },
    Case {
        sample_rate: 24_000,
        channels: 1,
        bitrate: 32,
        stereo: StereoMode::Joint,
        min_snr_db: 23.0,
        noise_shaping: NoiseShaping::Off,
    },
    // Outer loop with scalefactors (M4, opt-in): exercises scalefac_compress,
    // preflag and scalefac_scale through real decoders.
    Case {
        sample_rate: 44_100,
        channels: 2,
        bitrate: 320,
        stereo: StereoMode::Stereo,
        min_snr_db: 37.0,
        noise_shaping: NoiseShaping::AbsoluteThreshold,
    },
    Case {
        sample_rate: 32_000,
        channels: 1,
        bitrate: 96,
        stereo: StereoMode::Joint,
        min_snr_db: 33.0,
        noise_shaping: NoiseShaping::AbsoluteThreshold,
    },
];

const SECONDS: usize = 4;

/// A deterministic music-like test signal: a few harmonic tones with slow
/// vibrato and decaying notes, plus low-level noise, peaking around -6 dBFS.
fn test_signal(sample_rate: u32, channel: usize, n: usize) -> Vec<f32> {
    let fs = sample_rate as f64;
    // original: 64-bit LCG (Knuth MMIX constants).
    let mut seed = 0x1234_5678 + channel as u64;
    (0..n)
        .map(|i| {
            let t = i as f64 / fs;
            let note = (t * 2.0).floor();
            let since = t - note / 2.0;
            let f0 =
                [220.0, 277.18, 329.63, 440.0][note as usize % 4] * (1.0 + 0.003 * (6.0 * t).sin());
            let env = (-3.0 * since).exp();
            let tone: f64 = (1..=6)
                .map(|h| {
                    (2.0 * std::f64::consts::PI * f0 * h as f64 * t + channel as f64).sin()
                        / h as f64
                })
                .sum();
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let noise = ((seed >> 40) as f64 / f64::from(1 << 23)) - 1.0;
            (0.25 * env * tone + 0.01 * noise) as f32
        })
        .collect()
}

struct Encoded {
    mp3: Vec<u8>,
    /// Input as coded (after any downmix), per coded channel.
    input: Vec<Vec<f32>>,
    spf: usize,
}

fn encode(case: &Case) -> Encoded {
    let n = case.sample_rate as usize * SECONDS;
    let planes: Vec<Vec<f32>> = (0..case.channels)
        .map(|c| test_signal(case.sample_rate, c, n))
        .collect();
    let mut config = EncoderConfig::cbr(case.sample_rate, case.channels, case.bitrate);
    config.stereo = case.stereo;
    config.noise_shaping = case.noise_shaping;
    let mut enc = Encoder::new(config).unwrap();
    let coded_channels = enc.channel_mode().channels();
    let spf = enc.stream_params().version.samples_per_frame();
    // Feed in uneven chunks to exercise buffering.
    let mut mp3 = Vec::new();
    for chunk in (0..n).step_by(4321) {
        let end = (chunk + 4321).min(n);
        let slices: Vec<&[f32]> = planes.iter().map(|p| &p[chunk..end]).collect();
        mp3.extend(enc.encode(&slices).unwrap());
    }
    mp3.extend(enc.finish().unwrap());
    let input = if coded_channels == planes.len() {
        planes
    } else {
        vec![
            planes[0]
                .iter()
                .zip(&planes[1])
                .map(|(l, r)| 0.5 * (l + r))
                .collect(),
        ]
    };
    Encoded { mp3, input, spf }
}

/// True if any MPEG-1 granule in the stream has non-zero
/// `scalefac_compress` or `preflag` (side-info layout per 11172-3 §2.4.1.7:
/// 9 + 5 + 4 bits (mono) or 9 + 3 + 8 bits (stereo) before the granules,
/// 59 bits per granule and channel, `scalefac_compress` at offset 29 and
/// `preflag` at offset 56 within each).
fn uses_scalefactors(mp3: &[u8], channels: usize) -> bool {
    const BITRATES: [usize; 15] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
    ];
    const RATES: [usize; 3] = [44_100, 48_000, 32_000];
    let bit = |data: &[u8], pos: usize| (data[pos / 8] >> (7 - pos % 8)) & 1;
    let field = |data: &[u8], pos: usize, n: usize| {
        (0..n).fold(0usize, |acc, k| {
            (acc << 1) | usize::from(bit(data, pos + k))
        })
    };
    let mut pos = 0;
    while pos + 4 <= mp3.len() {
        let h = &mp3[pos..];
        assert_eq!(
            (h[0], h[1] & 0xFE),
            (0xFF, 0xFA),
            "MPEG-1 Layer III, no CRC"
        );
        let len = 144 * 1000 * BITRATES[usize::from(h[2] >> 4)]
            / RATES[usize::from((h[2] >> 2) & 3)]
            + usize::from((h[2] >> 1) & 1);
        let side = &h[4..];
        let first = if channels == 1 { 18 } else { 20 };
        for g in 0..2 * channels {
            let at = first + 59 * g;
            if field(side, at + 29, 4) != 0 || bit(side, at + 56) == 1 {
                return true;
            }
        }
        pos += len;
    }
    false
}

fn label(case: &Case) -> String {
    format!(
        "{} Hz {}ch {} kbps {:?} {:?}",
        case.sample_rate, case.channels, case.bitrate, case.stereo, case.noise_shaping
    )
}

/// Check decoded interleaved 16-bit PCM against the input: sample count,
/// gain (least squares, after the filterbank delay) and SNR.
fn check_audio(case: &Case, enc: &Encoded, pcm: &[i16], decoder: &str) {
    let channels = enc.input.len();
    let n = enc.input[0].len();
    let decoded = pcm.len() / channels;
    let frames = (n + HybridFilterbank::DELAY).div_ceil(enc.spf);
    assert_eq!(
        decoded,
        frames * enc.spf,
        "{decoder} {}: decoded length",
        label(case)
    );

    let d = HybridFilterbank::DELAY;
    for (c, input) in enc.input.iter().enumerate() {
        let (mut xy, mut xx, mut yy) = (0.0f64, 0.0f64, 0.0f64);
        for (i, &x) in input.iter().enumerate() {
            let y = f64::from(pcm[(i + d) * channels + c]) / 32768.0;
            let x = f64::from(x);
            xy += x * y;
            xx += x * x;
            yy += y * y;
        }
        let gain = xy / xx;
        // Residual after removing the best-fit scaled input.
        let err = yy - 2.0 * gain * xy + gain * gain * xx;
        let snr = 10.0 * (xx * gain * gain / err).log10();
        eprintln!(
            "{decoder} {} ch{c}: gain {gain:.4}, SNR {snr:.1} dB",
            label(case)
        );
        assert!(
            (0.95..1.05).contains(&gain),
            "{decoder} {} ch{c}: gain {gain}",
            label(case)
        );
        assert!(
            snr >= case.min_snr_db,
            "{decoder} {} ch{c}: SNR {snr:.1} dB",
            label(case)
        );
    }
}

#[test]
fn minimp3_decodes_input() {
    for case in CASES {
        let enc = encode(case);
        if case.noise_shaping != NoiseShaping::Off {
            assert!(
                uses_scalefactors(&enc.mp3, enc.input.len()),
                "{}: no scalefactors in stream",
                label(case)
            );
        }
        let mut dec = rmp3::Decoder::new(&enc.mp3);
        let mut pcm = Vec::new();
        while let Some(frame) = dec.next() {
            let rmp3::Frame::Audio(audio) = frame else {
                panic!("{}: non-audio data in stream", label(case));
            };
            assert_eq!(audio.sample_rate(), case.sample_rate, "{}", label(case));
            assert_eq!(
                audio.channels() as usize,
                enc.input.len(),
                "{}",
                label(case)
            );
            assert_eq!(audio.bitrate(), case.bitrate, "{}", label(case));
            pcm.extend_from_slice(audio.samples());
        }
        check_audio(case, &enc, &pcm, "minimp3");
    }
}

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn ffmpeg_decodes_without_warnings() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not found; skipping ffmpeg decode test");
        return;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    for (i, case) in CASES.iter().enumerate() {
        let enc = encode(case);
        let path = dir.join(format!("decode_{i}.mp3"));
        std::fs::write(&path, &enc.mp3).unwrap();

        let out = Command::new("ffmpeg")
            .args(["-nostdin", "-v", "warning", "-i"])
            .arg(&path)
            .args(["-f", "s16le", "-acodec", "pcm_s16le", "-"])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "{}: ffmpeg failed: {stderr}",
            label(case)
        );
        // The demuxer warns about duration estimation on any raw stream
        // without a Xing/Info tag. That tag is M8; drop this exception then.
        let warnings: Vec<&str> = stderr
            .lines()
            .filter(|l| !l.contains("Estimating duration from bitrate"))
            .collect();
        assert!(
            warnings.is_empty(),
            "{}: ffmpeg warned: {stderr}",
            label(case)
        );

        let pcm: Vec<i16> = out
            .stdout
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        check_audio(case, &enc, &pcm, "ffmpeg");
    }
}
