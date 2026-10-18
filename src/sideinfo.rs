//! Layer III side information: the per-frame fields between the header (and
//! optional CRC) and the main data.
//!
//! Sources (see PROVENANCE.md):
//! - ISO/IEC 11172-3 §2.4.1.7 (syntax) and §2.4.2.7 (semantics): MPEG-1 layout,
//!   two granules, `scfsi`, 4-bit `scalefac_compress`, explicit `preflag`.
//! - ISO/IEC 13818-3 §2.4.1.7: LSF layout, one granule, no `scfsi`, 8-bit
//!   `main_data_begin`, 9-bit `scalefac_compress`, `preflag` not transmitted.

use crate::bitstream::BitWriter;
use crate::header::Version;

/// `block_type` for a granule that uses window switching (§2.4.2.7).
/// Normal long blocks (`block_type` 0) are signalled by
/// [`Window::Long`] instead, with `window_switching_flag` = 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    /// Long-to-short transition window (`block_type` 1).
    Start,
    /// Three short windows (`block_type` 2).
    Short,
    /// Short-to-long transition window (`block_type` 3).
    Stop,
}

impl BlockType {
    fn bits(self) -> u32 {
        match self {
            BlockType::Start => 1,
            BlockType::Short => 2,
            BlockType::Stop => 3,
        }
    }
}

/// The two variants of the 22 bits that follow `window_switching_flag`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    /// `window_switching_flag` = 0: normal long block with three big-value
    /// regions.
    Long {
        table_select: [u8; 3],
        region0_count: u8,
        region1_count: u8,
    },
    /// `window_switching_flag` = 1. Region boundaries are implicit, so only
    /// two Huffman tables are sent.
    Switched {
        block_type: BlockType,
        mixed_block_flag: bool,
        table_select: [u8; 2],
        subblock_gain: [u8; 3],
    },
}

impl Default for Window {
    fn default() -> Self {
        Window::Long {
            table_select: [0; 3],
            region0_count: 0,
            region1_count: 0,
        }
    }
}

/// Side information for one channel of one granule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GranuleInfo {
    /// Bits of main data (scalefactors + Huffman) for this granule/channel. 12 bits.
    pub part2_3_length: u16,
    /// Number of spectral value pairs in the big-values region. 9 bits, ≤ 288.
    pub big_values: u16,
    /// 8 bits.
    pub global_gain: u8,
    /// 4 bits for MPEG-1, 9 bits for LSF.
    pub scalefac_compress: u16,
    pub window: Window,
    /// MPEG-1 only. LSF derives it from `scalefac_compress`, so it is not
    /// written there and must be false.
    pub preflag: bool,
    pub scalefac_scale: bool,
    pub count1table_select: bool,
}

impl GranuleInfo {
    /// Silent granule: no main data at all, so the decoder reconstructs zeros.
    pub fn silent() -> Self {
        Self::default()
    }

    fn write(&self, version: Version, w: &mut BitWriter) {
        let lsf = version != Version::Mpeg1;
        w.put_bits(self.part2_3_length.into(), 12);
        w.put_bits(self.big_values.into(), 9);
        w.put_bits(self.global_gain.into(), 8);
        w.put_bits(self.scalefac_compress.into(), if lsf { 9 } else { 4 });
        match self.window {
            Window::Long {
                table_select,
                region0_count,
                region1_count,
            } => {
                w.put_bit(false); // window_switching_flag
                for t in table_select {
                    w.put_bits(t.into(), 5);
                }
                w.put_bits(region0_count.into(), 4);
                w.put_bits(region1_count.into(), 3);
            }
            Window::Switched {
                block_type,
                mixed_block_flag,
                table_select,
                subblock_gain,
            } => {
                w.put_bit(true); // window_switching_flag
                w.put_bits(block_type.bits(), 2);
                w.put_bit(mixed_block_flag);
                for t in table_select {
                    w.put_bits(t.into(), 5);
                }
                for g in subblock_gain {
                    w.put_bits(g.into(), 3);
                }
            }
        }
        if lsf {
            debug_assert!(!self.preflag, "LSF does not transmit preflag");
        } else {
            w.put_bit(self.preflag);
        }
        w.put_bit(self.scalefac_scale);
        w.put_bit(self.count1table_select);
    }
}

/// Side information for one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SideInfo {
    /// Negative offset in bytes from the frame's main-data start to where its
    /// main data begins (bit reservoir). 9 bits MPEG-1, 8 bits LSF.
    pub main_data_begin: u16,
    /// MPEG-1: 5 bits mono, 3 bits stereo. LSF: 1 bit mono, 2 bits stereo.
    pub private_bits: u8,
    /// Scalefactor selection information, `[ch][band group]`. MPEG-1 only.
    pub scfsi: [[bool; 4]; 2],
    /// `[granule][channel]`. LSF uses granule 0 only.
    pub granules: [[GranuleInfo; 2]; 2],
}

impl SideInfo {
    /// Side info for a frame with no main data.
    pub fn silent() -> Self {
        Self::default()
    }

    /// Serialise for the given version and channel count. Writes exactly
    /// `StreamParams::side_info_bytes(channels)` bytes.
    pub fn write(&self, version: Version, channels: usize, w: &mut BitWriter) {
        debug_assert!((1..=2).contains(&channels));
        let mono = channels == 1;
        match version {
            Version::Mpeg1 => {
                w.put_bits(self.main_data_begin.into(), 9);
                w.put_bits(self.private_bits.into(), if mono { 5 } else { 3 });
                for ch in &self.scfsi[..channels] {
                    for &band in ch {
                        w.put_bit(band);
                    }
                }
            }
            Version::Mpeg2 | Version::Mpeg25 => {
                w.put_bits(self.main_data_begin.into(), 8);
                w.put_bits(self.private_bits.into(), if mono { 1 } else { 2 });
            }
        }
        for gr in &self.granules[..version.granules()] {
            for g in &gr[..channels] {
                g.write(version, w);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::StreamParams;

    fn bytes(si: &SideInfo, version: Version, channels: usize) -> Vec<u8> {
        let mut w = BitWriter::new();
        si.write(version, channels, &mut w);
        w.into_bytes()
    }

    #[test]
    fn sizes_match_spec() {
        // Bit-exact sizes: 136/256 bits for MPEG-1, 72/136 for LSF, for both
        // window variants (each is 22 bits).
        let switched = GranuleInfo {
            window: Window::Switched {
                block_type: BlockType::Short,
                mixed_block_flag: false,
                table_select: [0; 2],
                subblock_gain: [0; 3],
            },
            ..GranuleInfo::default()
        };
        for (rate, version) in [(44_100, Version::Mpeg1), (22_050, Version::Mpeg2)] {
            let p = StreamParams::new(rate, 64).unwrap();
            for ch in 1..=2 {
                for g in [GranuleInfo::silent(), switched] {
                    let si = SideInfo {
                        granules: [[g; 2]; 2],
                        ..SideInfo::silent()
                    };
                    let mut w = BitWriter::new();
                    si.write(version, ch, &mut w);
                    assert_eq!(w.bit_len(), p.side_info_bytes(ch) * 8, "{version:?} {ch}ch");
                }
            }
        }
    }

    #[test]
    fn mpeg1_mono_field_positions() {
        // Hand-computed: 9 (main_data_begin) + 5 (private) + 4 (scfsi) = 18
        // bits precede granule 0. part2_3_length occupies bits 18..30.
        // Granule 1 starts at 18 + 59 = 77; its global_gain sits after
        // part2_3_length (12) and big_values (9), at bits 98..106.
        let mut si = SideInfo::silent();
        si.granules[0][0].part2_3_length = 0xFFF;
        si.granules[1][0].global_gain = 0xFF;
        let mut expect = vec![0u8; 17];
        expect[2] = 0x3F;
        expect[3] = 0xFC;
        expect[12] = 0x3F;
        expect[13] = 0xC0;
        assert_eq!(bytes(&si, Version::Mpeg1, 1), expect);
    }

    #[test]
    fn mpeg1_stereo_header_fields() {
        // main_data_begin (9) = 0x1FF, private (3) = 0b101, scfsi ch1 band 3.
        // Bits: 1111_1111 1101_0000 0001 ...
        let mut si = SideInfo::silent();
        si.main_data_begin = 0x1FF;
        si.private_bits = 0b101;
        si.scfsi[1][3] = true;
        let b = bytes(&si, Version::Mpeg1, 2);
        assert_eq!(b.len(), 32);
        assert_eq!(&b[..3], &[0xFF, 0xD0, 0x10]);
        assert!(b[3..].iter().all(|&x| x == 0));
    }

    #[test]
    fn lsf_mono_scalefac_compress_is_9_bits() {
        // 8 (main_data_begin) + 1 (private) + 12 + 9 + 8 = 38 bits precede
        // scalefac_compress, which then spans bits 38..47.
        let mut si = SideInfo::silent();
        si.granules[0][0].scalefac_compress = 0x1FF;
        let mut expect = vec![0u8; 9];
        expect[4] = 0x03;
        expect[5] = 0xFE;
        assert_eq!(bytes(&si, Version::Mpeg2, 1), expect);
    }

    #[test]
    fn switched_window_layout() {
        // MPEG-1 mono, granule 0. window_switching_flag is bit 18+33 = 51,
        // then block_type (2), mixed_block_flag (1), table_select (2×5),
        // subblock_gain (3×3).
        let mut si = SideInfo::silent();
        si.granules[0][0].window = Window::Switched {
            block_type: BlockType::Stop,
            mixed_block_flag: true,
            table_select: [0b10001, 0],
            subblock_gain: [0, 0, 0b111],
        };
        let b = bytes(&si, Version::Mpeg1, 1);
        // Bits 51..=60: 1, 11, 1, 10001, 0 -> byte 6 (bits 48..56) = 000_1_11_1_1,
        // byte 7 (bits 56..64) = 0001_0000.
        assert_eq!(b[6], 0b0001_1111);
        assert_eq!(b[7], 0b0001_0000);
        // table_select[1] is bits 60..65, subblock_gain bits 65..74, so the
        // last gain (0b111) is bits 71..74.
        assert_eq!(b[8], 0b0000_0001);
        assert_eq!(b[9], 0b1100_0000);
    }
}
