//! M1 exit criterion (docs/PLAN.md): a 10 s input encodes to a stream that
//! ffmpeg and minimp3 decode without warnings, to silence, with the right
//! duration.
//!
//! The ffmpeg half is skipped (with a note on stderr) when `ffmpeg` is not on
//! PATH. CI installs it.

use std::path::PathBuf;
use std::process::Command;

use mp3enc::{Encoder, EncoderConfig, StereoMode};

struct Case {
    sample_rate: u32,
    channels: usize,
    bitrate: u32,
    stereo: StereoMode,
}

const CASES: &[Case] = &[
    // 44.1 and 22.05 kHz exercise the padding bit; 48 and 32 kHz never pad
    // at these bitrates.
    Case {
        sample_rate: 44_100,
        channels: 2,
        bitrate: 128,
        stereo: StereoMode::Joint,
    },
    Case {
        sample_rate: 44_100,
        channels: 2,
        bitrate: 320,
        stereo: StereoMode::Stereo,
    },
    Case {
        sample_rate: 44_100,
        channels: 1,
        bitrate: 64,
        stereo: StereoMode::Joint,
    },
    Case {
        sample_rate: 48_000,
        channels: 2,
        bitrate: 192,
        stereo: StereoMode::Mono,
    },
    Case {
        sample_rate: 32_000,
        channels: 1,
        bitrate: 40,
        stereo: StereoMode::Joint,
    },
    // LSF (13818-3) side-info layout.
    Case {
        sample_rate: 22_050,
        channels: 2,
        bitrate: 64,
        stereo: StereoMode::Joint,
    },
    Case {
        sample_rate: 24_000,
        channels: 1,
        bitrate: 32,
        stereo: StereoMode::Joint,
    },
];

const SECONDS: usize = 10;

/// Encode 10 s of a 1 kHz tone (the content is irrelevant until M2, but it
/// proves the encoder ignores rather than leaks input in M1).
fn encode(case: &Case) -> (Vec<u8>, usize, usize, usize) {
    let n = case.sample_rate as usize * SECONDS;
    let tone: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / case.sample_rate as f32;
            0.5 * (2.0 * std::f32::consts::PI * 1000.0 * t).sin()
        })
        .collect();
    let planes: Vec<&[f32]> = vec![&tone; case.channels];

    let mut config = EncoderConfig::cbr(case.sample_rate, case.channels, case.bitrate);
    config.stereo = case.stereo;
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
    (mp3, n, coded_channels, spf)
}

fn label(case: &Case) -> String {
    format!(
        "{} Hz {}ch {} kbps {:?}",
        case.sample_rate, case.channels, case.bitrate, case.stereo
    )
}

#[test]
fn minimp3_decodes_to_silence() {
    for case in CASES {
        let (mp3, n, channels, spf) = encode(case);
        let mut dec = rmp3::Decoder::new(&mp3);
        let (mut frames, mut samples) = (0usize, 0usize);
        while let Some(frame) = dec.next() {
            let rmp3::Frame::Audio(audio) = frame else {
                panic!("{}: non-audio data in stream", label(case));
            };
            assert_eq!(audio.sample_rate(), case.sample_rate, "{}", label(case));
            assert_eq!(audio.channels() as usize, channels, "{}", label(case));
            assert_eq!(audio.bitrate(), case.bitrate, "{}", label(case));
            assert!(audio.samples().iter().all(|&s| s == 0), "{}", label(case));
            frames += 1;
            samples += audio.sample_count();
        }
        assert_eq!(frames, n.div_ceil(spf), "{}", label(case));
        assert_eq!(samples, frames * spf, "{}", label(case));
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
        let (mp3, n, channels, spf) = encode(case);
        let path = dir.join(format!("m1_silent_{i}.mp3"));
        std::fs::write(&path, &mp3).unwrap();

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
        assert!(pcm.iter().all(|&s| s == 0), "{}: not silent", label(case));

        // Duration: we emit ceil(n / spf) whole frames, the last one
        // zero-padded. Without a gapless (Info) tag ffmpeg trims nothing, so
        // it must return exactly that many samples.
        let decoded = pcm.len() / channels;
        assert_eq!(
            decoded,
            n.div_ceil(spf) * spf,
            "{}: decoded sample count",
            label(case)
        );
    }
}
