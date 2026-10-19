//! Clean-room MPEG-1/2 Audio Layer III (MP3) encoder.
//!
//! Status: M4. [`Encoder`] runs the hybrid filterbank and the rate loop. The
//! outer (distortion control) loop with scalefactors is available as
//! [`NoiseShaping::AbsoluteThreshold`] but off by default until the
//! psychoacoustic model (M5). No bit reservoir yet. See `docs/PLAN.md`.

#![forbid(unsafe_code)]

pub mod bitstream;
pub mod header;
#[cfg(test)]
mod refdec;
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
use header::{ChannelMode, FrameHeader, PaddingState, StreamParams, Version};
use mdct::{GRANULE_SAMPLES, HybridFilterbank};
use quantize::{Ath, CodedGranule, ScalefactorBands};
use sideinfo::{GranuleInfo, SideInfo, Window};

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

/// How the encoder decides where quantization noise may go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NoiseShaping {
    /// One global quantizer step per granule (rate loop only, no
    /// scalefactors). The default until the psychoacoustic model (M5).
    #[default]
    Off,
    /// Outer loop with scalefactors, using the absolute threshold of hearing
    /// as the allowed distortion (MPEG-1 only). Experimental: it scores
    /// below `Off` on the dev corpus; see `quantize::outer_loop`.
    AbsoluteThreshold,
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
    pub noise_shaping: NoiseShaping,
}

impl EncoderConfig {
    pub fn cbr(sample_rate: u32, channels: usize, bitrate_kbps: u32) -> Self {
        Self {
            sample_rate,
            channels,
            rate: RateControl::Cbr(bitrate_kbps),
            stereo: StereoMode::default(),
            noise_shaping: NoiseShaping::default(),
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
    /// One hybrid filterbank per coded channel.
    filterbanks: Vec<HybridFilterbank>,
    bands: ScalefactorBands,
    /// Allowed distortion for the outer loop: present only with
    /// [`NoiseShaping::AbsoluteThreshold`] at an MPEG-1 rate.
    ath: Option<Ath>,
    /// Input samples per channel received so far, and samples coded.
    samples_in: usize,
    samples_coded: usize,
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
        if params.version == Version::Mpeg25 {
            return Err(Error::NotImplemented(
                "MPEG-2.5 (no standard scalefactor band tables; optional in M8)",
            ));
        }
        let bands = ScalefactorBands::long(config.sample_rate)
            .ok_or(Error::UnsupportedSampleRate(config.sample_rate))?;
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
            filterbanks: vec![HybridFilterbank::new(); mode.channels()],
            ath: match config.noise_shaping {
                NoiseShaping::Off => None,
                NoiseShaping::AbsoluteThreshold => Ath::new(config.sample_rate, &bands),
            },
            bands,
            samples_in: 0,
            samples_coded: 0,
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
        self.samples_in += pcm[0].len();
        Ok(self.drain_frames())
    }

    /// Code every whole frame of pending samples.
    fn drain_frames(&mut self) -> Vec<u8> {
        let spf = self.params.version.samples_per_frame();
        let frames = self.pending[0].len() / spf;
        let mut out = Vec::with_capacity(frames * self.params.frame_bytes(true));
        for f in 0..frames {
            self.write_frame(f * spf, &mut out);
        }
        for buf in &mut self.pending {
            buf.drain(..frames * spf);
        }
        self.samples_coded += frames * spf;
        out
    }

    /// Flush and return the final frames.
    ///
    /// The filterbanks delay the signal by [`HybridFilterbank::DELAY`]
    /// samples (encoder plus decoder), so zeros are appended until every
    /// input sample has come out of a decoder. The decoded stream therefore
    /// starts with that many samples of delay and ends with padding;
    /// signalling them for gapless playback is M8.
    pub fn finish(mut self) -> Result<Vec<u8>, Error> {
        if self.samples_in == 0 {
            return Ok(Vec::new());
        }
        let spf = self.params.version.samples_per_frame();
        let target = (self.samples_in + HybridFilterbank::DELAY).div_ceil(spf) * spf;
        let pad = target - self.samples_coded - self.pending[0].len();
        for buf in &mut self.pending {
            buf.resize(buf.len() + pad, 0.0);
        }
        Ok(self.drain_frames())
    }

    /// Append one frame coding `samples_per_frame` pending samples starting
    /// at `offset`.
    ///
    /// M3: no bit reservoir (`main_data_begin` = 0), so each frame's main
    /// data must fit in the frame itself. The space after the header and side
    /// information is split evenly between granules and channels; unused
    /// bits at the end are zero ancillary data (11172-3 §2.4.1.7).
    fn write_frame(&mut self, offset: usize, out: &mut Vec<u8>) {
        let version = self.params.version;
        let channels = self.mode.channels();
        let header = FrameHeader::new(self.params, self.mode, self.padding.next_padding());
        let frame_bytes = header.frame_bytes();
        let main_bits = (frame_bytes - 4 - self.params.side_info_bytes(channels)) * 8;
        let slots = version.granules() * channels;
        // part2_3_length is a 12-bit field.
        let budget = ((main_bits / slots) as u32).min(4095);

        let mut side = SideInfo::silent();
        let mut coded: Vec<CodedGranule> = Vec::with_capacity(slots);
        let mut xr = [0.0f32; quantize::GRANULE_LINES];
        for gr in 0..version.granules() {
            for ch in 0..channels {
                let start = offset + gr * GRANULE_SAMPLES;
                let pcm: &[f32; GRANULE_SAMPLES] = self.pending[ch][start..start + GRANULE_SAMPLES]
                    .try_into()
                    .expect("whole granule");
                self.filterbanks[ch].granule(pcm, &mut xr);
                let c = match &self.ath {
                    Some(ath) => quantize::outer_loop(&xr, &self.bands, ath, budget),
                    None => CodedGranule::without_scalefactors(quantize::rate_loop(
                        &xr,
                        &self.bands,
                        budget,
                    )),
                };
                let q = &c.q;
                side.granules[gr][ch] = GranuleInfo {
                    part2_3_length: c.part2_3_length() as u16,
                    big_values: q.plan.big_values,
                    global_gain: q.global_gain,
                    scalefac_compress: c.scalefac_compress.into(),
                    window: Window::Long {
                        table_select: q.plan.table_select,
                        region0_count: q.plan.region0_count,
                        region1_count: q.plan.region1_count,
                    },
                    preflag: c.preflag,
                    scalefac_scale: c.scalefac_scale,
                    count1table_select: q.plan.count1_table_b,
                };
                coded.push(c);
            }
        }

        let mut w = BitWriter::with_capacity(frame_bytes);
        header.write(&mut w);
        side.write(version, channels, &mut w);
        debug_assert_eq!(w.bit_len(), (4 + self.params.side_info_bytes(channels)) * 8);
        // Main data in syntax order (11172-3 §2.4.1.7): for each granule,
        // for each channel, scalefactors then Huffman bits. Long blocks with
        // scfsi = 0: bands 0–10 at slen1 bits, bands 11–20 at slen2 bits.
        for c in &coded {
            let (slen1, slen2) = quantize::SLEN[usize::from(c.scalefac_compress)];
            for (b, &v) in c.scalefac.iter().enumerate() {
                w.put_bits(v.into(), if b < 11 { slen1 } else { slen2 }.into());
            }
            huffman::write(&mut w, &c.q.ix, &c.q.plan, &self.bands);
        }
        let mut frame = w.into_bytes();
        debug_assert!(frame.len() <= frame_bytes);
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
        assert!(matches!(
            Encoder::new(EncoderConfig::cbr(11_025, 1, 32)),
            Err(Error::NotImplemented(_))
        ));
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
        // Flushing covers the filterbank delay: ceil((12000 + 1057) / 1152).
        assert_eq!(lens.len(), 12);
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
