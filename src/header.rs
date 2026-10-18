//! Layer III frame header: tables, frame length, padding, serialisation.
//!
//! Sources (see PROVENANCE.md):
//! - ISO/IEC 11172-3 §2.4.1.3 and §2.4.2.3: header syntax and semantics,
//!   bitrate_index and sampling_frequency tables, frame length.
//! - ISO/IEC 13818-3 §2.4.2.3: low-sampling-frequency (LSF) bitrates and rates.
//! - MPEG-2.5 is a de facto extension (not ISO) that reuses the LSF bitrates
//!   at half the LSF sample rates and repurposes the 12th sync bit.

use crate::Error;
use crate::bitstream::BitWriter;

/// MPEG audio version, as signalled by the two bits after the 11-bit sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    /// ISO/IEC 11172-3: 32, 44.1, 48 kHz.
    Mpeg1,
    /// ISO/IEC 13818-3 LSF: 16, 22.05, 24 kHz.
    Mpeg2,
    /// Non-ISO extension: 8, 11.025, 12 kHz.
    Mpeg25,
}

impl Version {
    fn bits(self) -> u32 {
        match self {
            Version::Mpeg1 => 0b11,
            Version::Mpeg2 => 0b10,
            Version::Mpeg25 => 0b00,
        }
    }

    /// PCM samples per channel in one frame (2 granules for MPEG-1, 1 for LSF).
    pub fn samples_per_frame(self) -> usize {
        match self {
            Version::Mpeg1 => 1152,
            Version::Mpeg2 | Version::Mpeg25 => 576,
        }
    }

    /// Granules per frame.
    pub fn granules(self) -> usize {
        match self {
            Version::Mpeg1 => 2,
            Version::Mpeg2 | Version::Mpeg25 => 1,
        }
    }

    /// Frame-length coefficient: bytes = coeff * bitrate / sample_rate (+ padding).
    /// 144 = 1152 / 8 for MPEG-1, 72 = 576 / 8 for LSF.
    fn length_coeff(self) -> u64 {
        match self {
            Version::Mpeg1 => 144,
            Version::Mpeg2 | Version::Mpeg25 => 72,
        }
    }
}

/// Layer III bitrates in kbps, indexed by bitrate_index 1..=14
/// (index 0 = free format, 15 = forbidden). 11172-3 Table 2.4.2.3.
const BITRATES_MPEG1: [u32; 14] = [
    32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
];
/// LSF Layer III bitrates, 13818-3 §2.4.2.3. MPEG-2.5 uses the same table.
const BITRATES_LSF: [u32; 14] = [8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];

/// Sample rates indexed by sampling_frequency (0..=2; 3 is reserved).
const RATES_MPEG1: [u32; 3] = [44_100, 48_000, 32_000];
const RATES_MPEG2: [u32; 3] = [22_050, 24_000, 16_000];
const RATES_MPEG25: [u32; 3] = [11_025, 12_000, 8_000];

/// Channel mode field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMode {
    Stereo,
    /// Joint stereo; mode_extension carries the M/S and intensity flags.
    JointStereo {
        ms: bool,
        intensity: bool,
    },
    DualChannel,
    Mono,
}

impl ChannelMode {
    pub fn channels(self) -> usize {
        match self {
            ChannelMode::Mono => 1,
            _ => 2,
        }
    }

    fn bits(self) -> (u32, u32) {
        match self {
            ChannelMode::Stereo => (0b00, 0),
            ChannelMode::JointStereo { ms, intensity } => {
                // Layer III mode_extension: bit 1 = MS stereo, bit 0 = intensity stereo.
                (0b01, (u32::from(ms) << 1) | u32::from(intensity))
            }
            ChannelMode::DualChannel => (0b10, 0),
            ChannelMode::Mono => (0b11, 0),
        }
    }
}

/// Resolved stream parameters shared by every frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamParams {
    pub version: Version,
    pub sample_rate: u32,
    sample_rate_index: u32,
    pub bitrate_kbps: u32,
    bitrate_index: u32,
}

impl StreamParams {
    /// Validate a sample rate / bitrate pair and look up the table indices.
    pub fn new(sample_rate: u32, bitrate_kbps: u32) -> Result<Self, Error> {
        let (version, rate_index) = [
            (Version::Mpeg1, &RATES_MPEG1),
            (Version::Mpeg2, &RATES_MPEG2),
            (Version::Mpeg25, &RATES_MPEG25),
        ]
        .iter()
        .find_map(|(v, table)| {
            table
                .iter()
                .position(|&r| r == sample_rate)
                .map(|i| (*v, i as u32))
        })
        .ok_or(Error::UnsupportedSampleRate(sample_rate))?;

        let bitrates = match version {
            Version::Mpeg1 => &BITRATES_MPEG1,
            Version::Mpeg2 | Version::Mpeg25 => &BITRATES_LSF,
        };
        let bitrate_index = bitrates
            .iter()
            .position(|&b| b == bitrate_kbps)
            .map(|i| i as u32 + 1)
            .ok_or(Error::UnsupportedBitrate {
                bitrate_kbps,
                sample_rate,
            })?;

        Ok(Self {
            version,
            sample_rate,
            sample_rate_index: rate_index,
            bitrate_kbps,
            bitrate_index,
        })
    }

    /// Frame length in bytes, including the header, for a given padding bit.
    pub fn frame_bytes(&self, padding: bool) -> usize {
        let base = self.version.length_coeff() * u64::from(self.bitrate_kbps) * 1000
            / u64::from(self.sample_rate);
        base as usize + usize::from(padding)
    }

    /// Size of side information in bytes (11172-3 §2.4.1.7, 13818-3 §2.4.1.7).
    pub fn side_info_bytes(&self, channels: usize) -> usize {
        match (self.version, channels) {
            (Version::Mpeg1, 1) => 17,
            (Version::Mpeg1, _) => 32,
            (_, 1) => 9,
            (_, _) => 17,
        }
    }
}

/// Decides the padding bit frame by frame so the average frame length equals
/// `coeff * bitrate / sample_rate` exactly. Original work: a Bresenham-style
/// remainder accumulator, consistent with the padding semantics in §2.4.2.3.
#[derive(Debug, Clone)]
pub struct PaddingState {
    remainder_per_frame: u64,
    sample_rate: u64,
    acc: u64,
}

impl PaddingState {
    pub fn new(params: &StreamParams) -> Self {
        let numer = params.version.length_coeff() * u64::from(params.bitrate_kbps) * 1000;
        let sr = u64::from(params.sample_rate);
        Self {
            remainder_per_frame: numer % sr,
            sample_rate: sr,
            acc: 0,
        }
    }

    /// Returns the padding bit for the next frame.
    pub fn next_padding(&mut self) -> bool {
        self.acc += self.remainder_per_frame;
        if self.acc >= self.sample_rate {
            self.acc -= self.sample_rate;
            true
        } else {
            false
        }
    }
}

/// One frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub params: StreamParams,
    pub mode: ChannelMode,
    pub padding: bool,
    /// When true a 16-bit CRC follows the header. We don't emit CRCs yet.
    pub crc: bool,
    pub private: bool,
    pub copyright: bool,
    pub original: bool,
}

impl FrameHeader {
    pub fn new(params: StreamParams, mode: ChannelMode, padding: bool) -> Self {
        Self {
            params,
            mode,
            padding,
            crc: false,
            private: false,
            copyright: false,
            original: true,
        }
    }

    pub fn frame_bytes(&self) -> usize {
        self.params.frame_bytes(self.padding)
    }

    /// Serialise the 32-bit header (11172-3 §2.4.1.3).
    pub fn write(&self, w: &mut BitWriter) {
        let (mode, mode_ext) = self.mode.bits();
        w.put_bits(0x7FF, 11); // syncword (11 bits; 12th bit is part of `version` for 2.5)
        w.put_bits(self.params.version.bits(), 2);
        w.put_bits(0b01, 2); // layer: '01' = Layer III
        w.put_bit(!self.crc); // protection_bit: 1 = no CRC
        w.put_bits(self.params.bitrate_index, 4);
        w.put_bits(self.params.sample_rate_index, 2);
        w.put_bit(self.padding);
        w.put_bit(self.private);
        w.put_bits(mode, 2);
        w.put_bits(mode_ext, 2);
        w.put_bit(self.copyright);
        w.put_bit(self.original);
        w.put_bits(0b00, 2); // emphasis: none
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_bytes_mpeg1_128k_joint_ms() {
        let p = StreamParams::new(44_100, 128).unwrap();
        let h = FrameHeader::new(
            p,
            ChannelMode::JointStereo {
                ms: true,
                intensity: false,
            },
            false,
        );
        let mut w = BitWriter::new();
        h.write(&mut w);
        assert_eq!(w.into_bytes(), vec![0xFF, 0xFB, 0x90, 0x64]);
    }

    #[test]
    fn header_bytes_mpeg2_mono() {
        // 22.05 kHz, 64 kbps (LSF index 8), mono, no CRC, original.
        let p = StreamParams::new(22_050, 64).unwrap();
        let h = FrameHeader::new(p, ChannelMode::Mono, false);
        let mut w = BitWriter::new();
        h.write(&mut w);
        assert_eq!(w.into_bytes(), vec![0xFF, 0xF3, 0x80, 0xC4]);
    }

    #[test]
    fn frame_lengths() {
        let p = StreamParams::new(44_100, 128).unwrap();
        assert_eq!(p.frame_bytes(false), 417);
        assert_eq!(p.frame_bytes(true), 418);
        let p = StreamParams::new(48_000, 320).unwrap();
        assert_eq!(p.frame_bytes(false), 960);
        let p = StreamParams::new(22_050, 64).unwrap();
        assert_eq!(p.frame_bytes(false), 208);
    }

    #[test]
    fn padding_hits_exact_average() {
        // Over 44100 frames, 128 kbps at 44.1 kHz must total exactly 144 * 128000 bytes.
        let p = StreamParams::new(44_100, 128).unwrap();
        let mut pad = PaddingState::new(&p);
        let total: usize = (0..44_100).map(|_| p.frame_bytes(pad.next_padding())).sum();
        assert_eq!(total, 144 * 128_000);
    }

    #[test]
    fn no_padding_needed_at_48k() {
        let p = StreamParams::new(48_000, 192).unwrap();
        let mut pad = PaddingState::new(&p);
        assert!((0..1000).all(|_| !pad.next_padding()));
    }

    #[test]
    fn rejects_invalid_combinations() {
        assert!(matches!(
            StreamParams::new(44_100, 8),
            Err(Error::UnsupportedBitrate { .. })
        ));
        assert!(matches!(
            StreamParams::new(22_050, 320),
            Err(Error::UnsupportedBitrate { .. })
        ));
        assert!(matches!(
            StreamParams::new(96_000, 128),
            Err(Error::UnsupportedSampleRate(96_000))
        ));
    }

    #[test]
    fn side_info_sizes() {
        let p1 = StreamParams::new(44_100, 128).unwrap();
        assert_eq!(p1.side_info_bytes(1), 17);
        assert_eq!(p1.side_info_bytes(2), 32);
        let p2 = StreamParams::new(24_000, 64).unwrap();
        assert_eq!(p2.side_info_bytes(1), 9);
        assert_eq!(p2.side_info_bytes(2), 17);
    }
}
