//! Clean-room MPEG-1/2 Audio Layer III (MP3) encoder.
//!
//! Status: M0 (scaffolding). The public API is in place, but [`Encoder::encode`]
//! returns [`Error::NotImplemented`] until milestone M3. See `docs/PLAN.md`.

#![forbid(unsafe_code)]

pub mod bitstream;
pub mod header;
pub mod wav;

// Encoder stages, filled in milestone by milestone (docs/PLAN.md).
pub mod filterbank;
pub mod huffman;
pub mod mdct;
pub mod psy;
pub mod quantize;
pub mod reservoir;
pub mod stereo;

use header::{ChannelMode, StreamParams};

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
        Err(Error::NotImplemented(
            "frame encoding (M1 side info, M2 filterbank/MDCT, M3 quantization)",
        ))
    }

    /// Flush buffered samples and return the final frames.
    pub fn finish(self) -> Result<Vec<u8>, Error> {
        Err(Error::NotImplemented("flush (M1)"))
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
}
