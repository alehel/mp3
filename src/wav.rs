//! Minimal RIFF/WAVE reader: PCM 16/24/32-bit integer and 32-bit float,
//! including WAVE_FORMAT_EXTENSIBLE. Original work from the public RIFF WAVE
//! format description (see PROVENANCE.md).

use crate::Error;

const FORMAT_PCM: u16 = 0x0001;
const FORMAT_FLOAT: u16 = 0x0003;
const FORMAT_EXTENSIBLE: u16 = 0xFFFE;

/// Decoded audio: one `Vec<f32>` per channel, samples in [-1.0, 1.0].
#[derive(Debug, Clone, PartialEq)]
pub struct Wav {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

impl Wav {
    pub fn num_frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }
}

fn u16_le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Parse a WAV file from memory.
pub fn parse(data: &[u8]) -> Result<Wav, Error> {
    let bad = |msg: &str| Error::InvalidWav(msg.to_string());
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(bad("not a RIFF/WAVE file"));
    }

    let mut pos = 12;
    let mut fmt: Option<(u16, u16, u32, u16)> = None; // (format, channels, rate, bits)
    let mut pcm: Option<&[u8]> = None;

    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32_le(data, pos + 4) as usize;
        let body_start = pos + 8;
        // Tolerate a truncated final data chunk (common with streamed writers).
        let body_end = (body_start + size).min(data.len());
        let body = &data[body_start..body_end];

        match id {
            b"fmt " => {
                if body.len() < 16 {
                    return Err(bad("fmt chunk too short"));
                }
                let mut format = u16_le(body, 0);
                let channels = u16_le(body, 2);
                let rate = u32_le(body, 4);
                let bits = u16_le(body, 14);
                if format == FORMAT_EXTENSIBLE {
                    // cbSize(2) validBits(2) channelMask(4) then the SubFormat GUID,
                    // whose first two bytes are the actual format tag.
                    if body.len() < 26 {
                        return Err(bad("extensible fmt chunk too short"));
                    }
                    format = u16_le(body, 24);
                }
                fmt = Some((format, channels, rate, bits));
            }
            b"data" => pcm = Some(body),
            _ => {}
        }
        // Chunks are word-aligned: odd sizes carry one pad byte.
        pos = body_start + size + (size & 1);
    }

    let (format, channels, sample_rate, bits) = fmt.ok_or_else(|| bad("missing fmt chunk"))?;
    let pcm = pcm.ok_or_else(|| bad("missing data chunk"))?;
    if channels == 0 {
        return Err(bad("zero channels"));
    }
    let channels = channels as usize;

    let decode: fn(&[u8]) -> f32 = match (format, bits) {
        (FORMAT_PCM, 16) => |s| f32::from(i16::from_le_bytes([s[0], s[1]])) / 32_768.0,
        (FORMAT_PCM, 24) => |s| {
            let v = i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8;
            v as f32 / 8_388_608.0
        },
        (FORMAT_PCM, 32) => |s| {
            let v = i32::from_le_bytes([s[0], s[1], s[2], s[3]]);
            (f64::from(v) / 2_147_483_648.0) as f32
        },
        (FORMAT_FLOAT, 32) => |s| f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
        _ => {
            return Err(Error::InvalidWav(format!(
                "unsupported sample format (tag {format:#06x}, {bits} bits)"
            )));
        }
    };

    let bytes_per_sample = bits as usize / 8;
    let frame_bytes = bytes_per_sample * channels;
    let frames = pcm.len() / frame_bytes;
    let mut out = vec![Vec::with_capacity(frames); channels];
    for frame in pcm.chunks_exact(frame_bytes) {
        for (ch, sample) in frame.chunks_exact(bytes_per_sample).enumerate() {
            out[ch].push(decode(sample));
        }
    }

    Ok(Wav {
        sample_rate,
        channels: out,
    })
}

/// Read and parse a WAV file from disk.
pub fn read(path: impl AsRef<std::path::Path>) -> Result<Wav, Error> {
    let data = std::fs::read(path)?;
    parse(&data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_wav(format: u16, channels: u16, rate: u32, bits: u16, pcm: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
        v.extend_from_slice(b"WAVE");
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&format.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        let block = channels * bits / 8;
        v.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
        v.extend_from_slice(&block.to_le_bytes());
        v.extend_from_slice(&bits.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        v.extend_from_slice(pcm);
        v
    }

    #[test]
    fn parses_16bit_stereo() {
        let mut pcm = Vec::new();
        for s in [0i16, -32768, 16384, 32767] {
            pcm.extend_from_slice(&s.to_le_bytes());
        }
        let w = parse(&make_wav(FORMAT_PCM, 2, 44_100, 16, &pcm)).unwrap();
        assert_eq!(w.sample_rate, 44_100);
        assert_eq!(w.num_frames(), 2);
        assert_eq!(w.channels[0], vec![0.0, 0.5]);
        assert_eq!(w.channels[1][0], -1.0);
        assert!((w.channels[1][1] - 32767.0 / 32768.0).abs() < 1e-7);
    }

    #[test]
    fn parses_24bit_negative() {
        let pcm = [0x00, 0x00, 0x80]; // -8388608 -> -1.0
        let w = parse(&make_wav(FORMAT_PCM, 1, 48_000, 24, &pcm)).unwrap();
        assert_eq!(w.channels[0], vec![-1.0]);
    }

    #[test]
    fn parses_float() {
        let mut pcm = Vec::new();
        pcm.extend_from_slice(&0.25f32.to_le_bytes());
        let w = parse(&make_wav(FORMAT_FLOAT, 1, 32_000, 32, &pcm)).unwrap();
        assert_eq!(w.channels[0], vec![0.25]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"not a wav file at all").is_err());
        assert!(parse(&make_wav(FORMAT_PCM, 1, 44_100, 8, &[0])).is_err());
    }
}
