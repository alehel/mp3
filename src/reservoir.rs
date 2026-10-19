//! Bit reservoir: `main_data_begin` and cross-frame bit borrowing.
//!
//! Sources (see PROVENANCE.md):
//! - ISO/IEC 11172-3 DIS §2.4.2.7 (`main_data_begin`): "a negative offset in
//!   bytes from the first byte of the audio sync word. The number of bytes
//!   belonging to the header and side information is not taken into
//!   account." So a frame's main data starts `main_data_begin` bytes before
//!   the end of its own side information, counting only main-data bytes of
//!   earlier frames.
//! - 11172-3 DIS §2.4.3.4, "Buffer considerations": the decoder buffer is 7680
//!   bits, so the reservoir may hold at most 7680 bits minus one mean frame;
//!   at 320 kbit/s that leaves nothing and `main_data_begin` is always 0.
//!   The 9-bit field (8 bits in 13818-3 LSF) caps it at 511 (255) bytes. All
//!   main data of a frame must have arrived by the next header.
//! - 11172-3 CD rev 1, Annex C.1.5.4 §2.2 "Bit reservoir control": bits a
//!   granule leaves unused go to the reservoir; borrowing is driven by the
//!   granule's demand; content above 0.8 of the maximum is released; excess
//!   over the maximum is written as stuffing.

use std::collections::VecDeque;

use crate::header::{StreamParams, Version};

/// Decoder input buffer, in bits (11172-3 §2.4.3.4, "Buffer considerations").
const BUFFER_BITS: usize = 7680;

/// Reservoir state plus the frames whose main-data space may still receive
/// bytes from later frames.
#[derive(Debug)]
pub struct Reservoir {
    /// Maximum content in bytes (see module docs).
    capacity: usize,
    /// Frames not yet output: complete frame bytes and how many bytes of each
    /// are filled (header, side information and main data so far).
    frames: VecDeque<(Vec<u8>, usize)>,
    /// Unfilled main-data bytes across `frames`: the reservoir content, and
    /// the next frame's `main_data_begin`.
    content: usize,
    /// Running average of frame demand (see [`Reservoir::budgets`]).
    typical: f32,
}

impl Reservoir {
    pub fn new(params: &StreamParams) -> Self {
        let field_max = match params.version {
            Version::Mpeg1 => 511,
            Version::Mpeg2 | Version::Mpeg25 => 255,
        };
        // Mean frame length in bits (§2.4.2.3), without padding.
        let spf = params.version.samples_per_frame();
        let mean_bits = params.bitrate_kbps as usize * 1000 * spf / params.sample_rate as usize;
        let capacity = (BUFFER_BITS.saturating_sub(mean_bits) / 8).min(field_max);
        Self {
            capacity,
            frames: VecDeque::new(),
            content: 0,
            typical: 0.0,
        }
    }

    /// Budgets for the next frame's granule-channels ([`allocate`]), judging
    /// its demand against the running average of earlier frames' demand,
    /// which is then updated.
    pub fn budgets(&mut self, demand: &[f32], main_bytes: usize) -> Vec<u32> {
        let total: f32 = demand.iter().sum();
        let typical = if self.typical > 0.0 {
            self.typical
        } else {
            total
        };
        let budgets = allocate(demand, typical, main_bytes, self.content, self.capacity);
        self.typical = typical + DEMAND_SMOOTHING * (total - typical);
        budgets
    }

    /// Maximum reservoir content in bytes.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Bytes available from earlier frames: the `main_data_begin` of the
    /// next frame.
    pub fn content(&self) -> usize {
        self.content
    }

    /// Add one frame. `frame` holds the header and side information and is
    /// `frame_bytes` long when complete; `main` is its main data, which
    /// starts in the unfilled tail of earlier frames (`main_data_begin` =
    /// [`content`](Self::content) when the side information was written)
    /// and must fit in that plus the frame's own space. Returns the frames
    /// that are now complete, in order.
    ///
    /// Any space left beyond [`capacity`](Self::capacity) is filled with zero
    /// stuffing (ancillary data, §2.4.1.7), so the next frame never points
    /// further back than the decoder can buffer.
    pub fn push(&mut self, mut frame: Vec<u8>, frame_bytes: usize, main: &[u8]) -> Vec<u8> {
        let own = frame_bytes - frame.len();
        assert!(
            main.len() <= self.content + own,
            "main data ({} bytes) exceeds reservoir ({}) plus frame ({own})",
            main.len(),
            self.content
        );
        let filled = frame.len();
        frame.resize(frame_bytes, 0);
        self.frames.push_back((frame, filled));
        self.content += own;
        self.fill(main);
        let stuffing = self.content.saturating_sub(self.capacity);
        self.fill(&vec![0; stuffing]);
        let mut out = Vec::new();
        while self.frames.front().is_some_and(|(f, n)| *n == f.len()) {
            out.extend(self.frames.pop_front().expect("checked").0);
        }
        out
    }

    /// Output every remaining frame; unfilled space stays zero.
    pub fn flush(&mut self) -> Vec<u8> {
        self.content = 0;
        self.frames.drain(..).flat_map(|(f, _)| f).collect()
    }

    /// Write `data` into the oldest unfilled main-data bytes.
    fn fill(&mut self, mut data: &[u8]) {
        self.content -= data.len();
        for (frame, filled) in &mut self.frames {
            if data.is_empty() {
                break;
            }
            let n = (frame.len() - *filled).min(data.len());
            frame[*filled..*filled + n].copy_from_slice(&data[..n]);
            *filled += n;
            data = &data[n..];
        }
        debug_assert!(data.is_empty());
    }
}

/// Bits for each granule-channel of one frame, in syntax order.
///
/// `demand` is an estimate of the bits each granule-channel needs (see
/// `Encoder::write_frame`), `typical` the usual total demand of a frame,
/// `main_bytes` the frame's own main-data space and `content` the
/// reservoir content before the frame. Returns the budgets; their sum never
/// exceeds `(content + main_bytes) · 8`, and is at least what keeps the
/// reservoir from overflowing.
///
/// Annex C.1.5.4 §2.2, with one reading resolved: the draft says
/// "max(more_bits/8, 0.6·main_data_end) bytes are taken from the bit
/// reservoir". With max, every granule asking for 100 bits more would empty
/// 60% of the reservoir regardless of how much it asked for, which makes the
/// demand meaningless; we read it as min (take what is asked, at most 60%),
/// the same kind of MIN/MAX slip as in the draft's pre-echo control (see
/// `psy.rs`). The rules are applied per frame rather than per granule, so a
/// frame's demand is the sum over its granule-channels.
///
/// original, two changes. (1) The draft's `more_bits = 3.1·PE − mean_bits`
/// needs a demand measured in real bits; ours (`demand_bits`) is off by a
/// signal-dependent factor (typically 0.5–1× the mean at 128 kbit/s), so
/// `more_bits` is instead the mean scaled by how far the frame's demand is
/// above or below `typical`. An absolute reading, tried first (run
/// `m7a-res-ms`), almost never borrowed and scored 4.537 against 4.543 at
/// 128 kbit/s. Driving `more_bits` from psychoacoustic model 2's PE
/// instead (relative, run `m7z-pe-rel`, or the draft's absolute
/// `3.1·PE − mean`, `m7z-pe-abs`) lowered the pre-echo proxy by 2–3 dB on
/// castanets, claves and glockenspiel but scored 4.536 / 4.538; kept as an
/// open question for the M7 listen. (2) A frame asking for less than typical gets
/// [`SAVE_FRACTION`] of its shortfall less, which is how the reservoir fills
/// (the draft relies on the loops leaving bits unused, which our rate loop
/// never does).
///
/// Within the frame, bits are shared in proportion to demand. Sharing them
/// equally instead (run `m7y-rel-even`) scored 4.508 but gave 1.6 dB more
/// side-signal SNR: in mid/side frames the side channel's lower demand
/// moves bits to the mid channel, which the mono-downmix metrics favour.
/// Demand follows the allowed distortion, which is the perceptual
/// criterion, so proportional sharing stays; the stereo listen (M7 🎧)
/// checks it.
pub fn allocate(
    demand: &[f32],
    typical: f32,
    main_bytes: usize,
    content: usize,
    capacity: usize,
) -> Vec<u32> {
    let mean = (main_bytes * 8) as f32;
    let reservoir = (content * 8) as f32;
    let total_demand: f32 = demand.iter().sum();
    let more = if typical > 0.0 {
        mean * (total_demand / typical - 1.0)
    } else {
        0.0
    };
    let mut target = if more > MORE_BITS_THRESHOLD {
        mean + more.min(BORROW_FRACTION * reservoir)
    } else if more < 0.0 {
        mean + SAVE_FRACTION * more
    } else {
        mean
    };
    // Release content above 0.8 of the maximum (§2.2), and never overflow:
    // what is not spent beyond `capacity` would be stuffing.
    let release = reservoir - RELEASE_LEVEL * (capacity * 8) as f32;
    if release > 0.0 {
        target = target.max(mean + release);
    }
    let floor = (content + main_bytes).saturating_sub(capacity) as f32 * 8.0;
    let ceiling = mean + reservoir;
    let target = target.clamp(floor, ceiling);
    // Share in proportion to demand (equally when there is none), at most
    // 4095 bits each: part2_3_length is a 12-bit field (§2.4.1.7).
    let n = demand.len() as f32;
    let budgets: Vec<u32> = demand
        .iter()
        .map(|&d| {
            let share = if total_demand > 0.0 {
                d / total_demand
            } else {
                1.0 / n
            };
            ((target * share) as u32).min(4095)
        })
        .collect();
    budgets
}

/// Extra demand (bits) below which nothing is borrowed (Annex C.1.5.4
/// §2.2: "If more_bits is greater than 100 bits").
const MORE_BITS_THRESHOLD: f32 = 100.0;

/// Largest share of the reservoir one frame may borrow (§2.2, "0.6 ·
/// main_data_end").
const BORROW_FRACTION: f32 = 0.6;

/// Reservoir level above which the excess is released (§2.2, "more than 0.8
/// times the maximum allowed content").
const RELEASE_LEVEL: f32 = 0.8;

/// Weight of the newest frame in the running average of demand (an
/// exponential average over roughly the last 4 frames, 0.1 s). original;
/// eval runs `m7y-rel-a0.25` (dev 128 kbps ViSQOL 4.543) vs `m7y-rel-s0.5`
/// (1/16: 4.541).
pub const DEMAND_SMOOTHING: f32 = 0.25;

/// Fraction of an easy frame's shortfall that is saved. original; eval runs
/// `m7y-rel-s0`, `-s0.5`, `-s1` (dev 128 kbps ViSQOL 4.540 / 4.541 / 4.539;
/// 0.5 also had the best pre-echo proxy at 192 kbps).
pub const SAVE_FRACTION: f32 = 0.5;

#[cfg(test)]
mod tests {
    use super::*;

    fn params(rate: u32, kbps: u32) -> StreamParams {
        StreamParams::new(rate, kbps).unwrap()
    }

    #[test]
    fn capacity_follows_buffer_and_field_limits() {
        // 320 kbit/s at 48 kHz: mean frame = 7680 bits, so no reservoir
        // (§2.4.3.4). 44.1 kHz frames are even longer.
        assert_eq!(Reservoir::new(&params(48_000, 320)).capacity(), 0);
        assert_eq!(Reservoir::new(&params(44_100, 320)).capacity(), 0);
        // 256 kbit/s at 48 kHz: (7680 − 6144) / 8 = 192 bytes.
        assert_eq!(Reservoir::new(&params(48_000, 256)).capacity(), 192);
        // 128 kbit/s at 44.1 kHz: 7680 − 3343 = 4337 bits → 542 bytes,
        // capped by the 9-bit field.
        assert_eq!(Reservoir::new(&params(44_100, 128)).capacity(), 511);
        // LSF, 8-bit field.
        assert_eq!(Reservoir::new(&params(22_050, 64)).capacity(), 255);
    }

    #[test]
    fn main_data_spills_back_into_earlier_frames() {
        let mut r = Reservoir {
            capacity: 100,
            frames: VecDeque::new(),
            content: 0,
            typical: 0.0,
        };
        // Frame A: 2 header bytes, 8 bytes of space, uses 3.
        let out = r.push(vec![0xAA, 0xAA], 10, &[1, 2, 3]);
        assert!(out.is_empty(), "A still has space for later frames");
        assert_eq!(r.content(), 5);
        // Frame B uses 5 bytes of A plus 6 of its own 8.
        let out = r.push(
            vec![0xBB, 0xBB],
            10,
            &[4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14],
        );
        assert_eq!(out, [0xAA, 0xAA, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(r.content(), 2);
        assert_eq!(r.flush(), [0xBB, 0xBB, 9, 10, 11, 12, 13, 14, 0, 0]);
    }

    #[test]
    fn overflow_becomes_stuffing() {
        let mut r = Reservoir {
            capacity: 3,
            frames: VecDeque::new(),
            content: 0,
            typical: 0.0,
        };
        // 8 bytes of space, 1 used: 7 left, of which 4 must be stuffed.
        let out = r.push(vec![0xAA, 0xAA], 10, &[9]);
        assert_eq!(r.content(), 3);
        assert!(out.is_empty());
        let out = r.push(vec![0xBB, 0xBB], 10, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
        assert_eq!(out.len(), 20);
        assert_eq!(&out[2..10], &[9, 0, 0, 0, 0, 1, 2, 3]);
        assert_eq!(r.content(), 0);
    }

    #[test]
    #[should_panic(expected = "exceeds reservoir")]
    fn cannot_borrow_more_than_available() {
        let mut r = Reservoir::new(&params(44_100, 128));
        r.push(vec![0; 36], 417, &vec![0; 382]);
    }

    #[test]
    fn allocation_respects_bounds() {
        // Demand is judged against the typical frame demand: 2.5× typical
        // asks for 1.5 × the mean (3600 bits) more. With 400 bytes in the
        // reservoir it borrows at most 60% of them.
        let b = allocate(&[3000.0, 3000.0], 2400.0, 300, 400, 511);
        let sum: u32 = b.iter().sum();
        assert_eq!(sum, 2400 + (0.6 * 3200.0) as u32);
        // Easy frame (half the typical demand): saves half the shortfall.
        let b = allocate(&[600.0, 600.0], 2400.0, 300, 0, 511);
        assert_eq!(b.iter().sum::<u32>(), 2400 - 600);
        // Easy frame near capacity: spends enough not to overflow.
        let b = allocate(&[600.0, 600.0], 2400.0, 300, 500, 511);
        assert!(b.iter().sum::<u32>() >= (500 + 300 - 511) * 8);
        // No reservoir (320 kbit/s): exactly the frame, at most 4095 each.
        let b = allocate(&[9000.0, 100.0], 2400.0, 1000, 0, 0);
        assert_eq!(b, [4095, (8000.0 * 100.0 / 9100.0) as u32]);
    }
}
