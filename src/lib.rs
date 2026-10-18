//! Clean-room MPEG-1/2 Audio Layer III (MP3) encoder.
//!
//! Status: M1. [`Encoder`] writes valid frames whose granules carry no
//! spectral data, so every stream decodes to silence. See `docs/PLAN.md`.

#![forbid(unsafe_code)]

pub mod bitstream;
pub mod header;
pub mod sideinfo;
pub mod wav;

// Encoder stages, filled in milestone by milestone (docs/PLAN.md).
pub mod filterbank;
pub mod huffman;
pub mod mdct;
pub mod psy;
pub mod quantize;
pub mod reservoir;
pub mod stereo;

use bitstream::BitWriter;
use header::{ChannelMode, FrameHeader, PaddingState, StreamParams};
use sideinfo::SideInfo;

/// Errors produced by the encoder and its I/O helpers.
#[derive(Debug)]
pub enum Error {
    UnsupportedSampleRate(u32),
    UnsupportedBitrate {
        bitrate_kbps: u32,
        sample_rate: u32,
    },
    UnsupportedChannels(usize),
    InvalidInput(String),
    InvalidWav(String),
    Io(std::io::Error),
    /// The named milestone hasn't been implemented yet.
    NotImplemented(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::UnsupportedSampleRate(r) => write!(f, "unsupported sample rate: {r} Hz"),
            Error::UnsupportedBitrate {
                bitrate_kbps,
                sample_rate,
            } => write!(
                f,
                "{bitrate_kbps} kbps is not a valid Layer III bitrate at {sample_rate} Hz"
            ),
            Error::UnsupportedChannels(n) => write!(f, "unsupported channel count: {n}"),
            Error::InvalidInput(m) => write!(f, "invalid input: {m}"),
            Error::InvalidWav(m) => write!(f, "invalid WAV: {m}"),
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::NotImplemented(m) => write!(f, "not implemented yet: {m}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// Stereo coding strategy requested by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StereoMode {
    /// Encoder chooses L/R or M/S per frame (needs M7; behaves as `Stereo` before).
    #[default]
    Joint,
    /// Independent left/right coding.
    Stereo,
    /// Downmix to one channel.
    Mono,
}

/// Rate control mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RateControl {
    /// Constant bitrate in kbps.
    Cbr(u32),
    /// Quality-targeted VBR (M8). 0 = best, 9 = smallest.
    Vbr { quality: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EncoderConfig {
    pub sample_rate: u32,
    pub channels: usize,
    pub rate: RateControl,
    pub stereo: StereoMode,
}

impl EncoderConfig {
    pub fn cbr(sample_rate: u32, channels: usize, bitrate_kbps: u32) -> Self {
        Self {
            sample_rate,
            channels,
            rate: RateControl::Cbr(bitrate_kbps),
            stereo: StereoMode::default(),
        }
    }
}

/// Streaming encoder. Feed PCM with [`encode`](Self::encode), then call
/// [`finish`](Self::finish) to flush.
#[derive(Debug)]
pub struct Encoder {
    config: EncoderConfig,
    params: StreamParams,
    mode: ChannelMode,
    padding: PaddingState,
    /// Samples not yet coded, one buffer per coded channel (after any downmix).
    pending: Vec<Vec<f32>>,
}

impl Encoder {
    pub fn new(config: EncoderConfig) -> Result<Self, Error> {
        if !(1..=2).contains(&config.channels) {
            return Err(Error::UnsupportedChannels(config.channels));
        }
        let bitrate = match config.rate {
            RateControl::Cbr(b) => b,
            RateControl::Vbr { .. } => return Err(Error::NotImplemented("VBR (M8)")),
        };
        let params = StreamParams::new(config.sample_rate, bitrate)?;
        let mode = match (config.channels, config.stereo) {
            (1, _) | (_, StereoMode::Mono) => ChannelMode::Mono,
            (_, StereoMode::Stereo) => ChannelMode::Stereo,
            (_, StereoMode::Joint) => ChannelMode::JointStereo {
                ms: false,
                intensity: false,
            },
        };
        Ok(Self {
            config,
            params,
            mode,
            padding: PaddingState::new(&params),
            pending: vec![Vec::new(); mode.channels()],
        })
    }

    pub fn config(&self) -> &EncoderConfig {
        &self.config
    }

    pub fn stream_params(&self) -> &StreamParams {
        &self.params
    }

    pub fn channel_mode(&self) -> ChannelMode {
        self.mode
    }

    /// Encode planar PCM (one slice per channel, samples in [-1, 1]) and return
    /// any complete MP3 frames produced.
    pub fn encode(&mut self, pcm: &[&[f32]]) -> Result<Vec<u8>, Error> {
        if pcm.len() != self.config.channels {
            return Err(Error::InvalidInput(format!(
                "expected {} channels, got {}",
                self.config.channels,
                pcm.len()
            )));
        }
        if pcm.windows(2).any(|w| w[0].len() != w[1].len()) {
            return Err(Error::InvalidInput("channel lengths differ".into()));
        }
        if self.pending.len() == pcm.len() {
            for (buf, input) in self.pending.iter_mut().zip(pcm) {
                buf.extend_from_slice(input);
            }
        } else {
            // Stereo input coded as mono: plain average of the two channels.
            self.pending[0].extend(pcm[0].iter().zip(pcm[1]).map(|(l, r)| 0.5 * (l + r)));
        }

        let spf = self.params.version.samples_per_frame();
        let frames = self.pending[0].len() / spf;
        let mut out = Vec::with_capacity(frames * self.params.frame_bytes(true));
        for _ in 0..frames {
            self.write_frame(&mut out);
        }
        for buf in &mut self.pending {
            buf.drain(..frames * spf);
        }
        Ok(out)
    }

    /// Flush buffered samples and return the final frames. A partial last
    /// frame is zero-padded to a whole frame.
    pub fn finish(mut self) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        if !self.pending[0].is_empty() {
            let spf = self.params.version.samples_per_frame();
            for buf in &mut self.pending {
                buf.resize(spf, 0.0);
            }
            self.write_frame(&mut out);
        }
        Ok(out)
    }

    /// Append one frame coding the first `samples_per_frame` pending samples.
    ///
    /// M1: every granule is silent (`part2_3_length` = 0), `main_data_begin`
    /// is 0 (no reservoir), and the rest of the frame after the side
    /// information is zero-filled. A decoder treats the unused main-data bytes
    /// as ancillary data (11172-3 §2.4.1.7, §2.4.2.7 `part2_3_length`).
    fn write_frame(&mut self, out: &mut Vec<u8>) {
        let header = FrameHeader::new(self.params, self.mode, self.padding.next_padding());
        let frame_bytes = header.frame_bytes();
        let mut w = BitWriter::with_capacity(frame_bytes);
        header.write(&mut w);
        SideInfo::silent().write(self.params.version, self.mode.channels(), &mut w);
        let mut frame = w.into_bytes();
        debug_assert_eq!(
            frame.len(),
            4 + self.params.side_info_bytes(self.mode.channels())
        );
        frame.resize(frame_bytes, 0);
        out.extend_from_slice(&frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_validation() {
        assert!(Encoder::new(EncoderConfig::cbr(44_100, 2, 192)).is_ok());
        assert!(matches!(
            Encoder::new(EncoderConfig::cbr(44_100, 3, 192)),
            Err(Error::UnsupportedChannels(3))
        ));
        assert!(Encoder::new(EncoderConfig::cbr(44_100, 2, 100)).is_err());
    }

    #[test]
    fn mono_input_forces_mono_mode() {
        let e = Encoder::new(EncoderConfig::cbr(48_000, 1, 128)).unwrap();
        assert_eq!(e.channel_mode(), ChannelMode::Mono);
    }

    #[test]
    fn encode_checks_channel_count() {
        let mut e = Encoder::new(EncoderConfig::cbr(44_100, 2, 128)).unwrap();
        let left = [0.0f32; 1152];
        assert!(matches!(e.encode(&[&left]), Err(Error::InvalidInput(_))));
    }

    /// Split a stream into frames by walking the header's frame lengths.
    fn frame_lengths(mp3: &[u8], params: &StreamParams) -> Vec<usize> {
        let mut lens = Vec::new();
        let mut pos = 0;
        while pos < mp3.len() {
            assert_eq!(&mp3[pos..pos + 2], &[0xFF, 0xFB], "sync at byte {pos}");
            let padding = mp3[pos + 2] & 0b10 != 0;
            let len = params.frame_bytes(padding);
            lens.push(len);
            pos += len;
        }
        assert_eq!(pos, mp3.len());
        lens
    }

    #[test]
    fn frames_follow_input_length_and_padding() {
        // 10 full frames fed in odd-sized chunks, then a partial frame.
        let mut e = Encoder::new(EncoderConfig::cbr(44_100, 2, 128)).unwrap();
        let chunk = vec![0.0f32; 1000];
        let mut mp3 = Vec::new();
        for _ in 0..12 {
            mp3.extend(e.encode(&[&chunk, &chunk]).unwrap());
        }
        // 12000 samples = 10 full frames + 480 buffered.
        assert_eq!(frame_lengths(&mp3, e.stream_params()).len(), 10);
        let params = *e.stream_params();
        mp3.extend(e.finish().unwrap());
        let lens = frame_lengths(&mp3, &params);
        assert_eq!(lens.len(), 11);
        // At 128 kbps / 44.1 kHz, frames are 417 or 418 bytes (§2.4.2.3).
        assert!(lens.iter().all(|&l| l == 417 || l == 418));
    }

    #[test]
    fn finish_with_no_input_is_empty() {
        let e = Encoder::new(EncoderConfig::cbr(48_000, 1, 64)).unwrap();
        assert!(e.finish().unwrap().is_empty());
    }

    #[test]
    fn stereo_input_downmixed_to_mono() {
        let mut config = EncoderConfig::cbr(48_000, 2, 64);
        config.stereo = StereoMode::Mono;
        let mut e = Encoder::new(config).unwrap();
        let x = [0.25f32; 1152];
        let mp3 = e.encode(&[&x, &x]).unwrap();
        assert_eq!(mp3.len(), e.stream_params().frame_bytes(false));
        // mode bits = '11' (single channel).
        assert_eq!(mp3[3] >> 6, 0b11);
    }
}
