//! Huffman coding of quantized spectra: region partitioning, table selection
//! and bit counting for the big_values, count1 and rzero regions.
//!
//! **Milestone:** M3. See docs/PLAN.md.
//!
//! **Allowed sources:** ISO/IEC 11172-3 §2.4.2.7, Annex B (Huffman tables B.7), Annex C.1.5.4 (region partitioning and table selection).
//!
//! **Must be tested by:** Round-trip every table: encode then decode with a test-only decoder built from the same Annex B tables.
//!
//! Clean-room reminder: derive from the sources above only. Cite clauses in
//! comments and log every source in PROVENANCE.md (CLEANROOM.md).
//!
//! Bitstream semantics follow the published ISO/IEC 13818-3:1994 text, which
//! restates the Layer III `huffmancodebits()` syntax and the `region0_count`
//! / `region1_count` / `big_values` / `count1` semantics in their final form
//! (§2.5.1, §2.5.2). The 11172-3 Committee Draft uses older semantics for
//! the region fields.

mod tables;

use crate::bitstream::BitWriter;
use crate::quantize::{GRANULE_LINES, ScalefactorBands};

/// Largest magnitude allowed in the big_values region (13818-3 §2.5.2,
/// `big_values` semantics).
pub const MAX_BIG_VALUE: u32 = 8191;

/// `linbits` for tables 0..31 (Table 3-B.7: 16–23 reuse table 16's codes,
/// 24–31 reuse table 24's).
const LINBITS: [u32; 32] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 6, 8, 10, 13, 4, 5, 6, 7, 8, 9, 11,
    13,
];

/// Tables usable for pair coding. Table 0 codes only zeros (no bits);
/// tables 4 and 14 are "not used" in Table 3-B.7.
const PAIR_TABLES: [usize; 29] = [
    1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28,
    29, 30, 31,
];

/// One big_values table: its codes, side length and `linbits`.
#[derive(Debug, Clone, Copy)]
pub struct PairTable {
    codes: &'static [(u8, u32)],
    size: usize,
    linbits: u32,
}

impl PairTable {
    /// Table `index` (0..=31). `None` for table 0 and the unused 4 and 14.
    pub fn get(index: usize) -> Option<Self> {
        let codes: &'static [(u8, u32)] = match index {
            1 => &tables::TABLE_1,
            2 => &tables::TABLE_2,
            3 => &tables::TABLE_3,
            5 => &tables::TABLE_5,
            6 => &tables::TABLE_6,
            7 => &tables::TABLE_7,
            8 => &tables::TABLE_8,
            9 => &tables::TABLE_9,
            10 => &tables::TABLE_10,
            11 => &tables::TABLE_11,
            12 => &tables::TABLE_12,
            13 => &tables::TABLE_13,
            15 => &tables::TABLE_15,
            16..=23 => &tables::TABLE_16,
            24..=31 => &tables::TABLE_24,
            _ => return None,
        };
        let size = codes.len().isqrt();
        Some(Self {
            codes,
            size,
            linbits: LINBITS[index],
        })
    }

    /// Largest magnitude this table can code: `size − 1`, or with an escape
    /// `15 + 2^linbits − 1` (13818-3 §2.5.2, `linbitsx`).
    pub fn max_value(&self) -> u32 {
        if self.linbits == 0 {
            self.size as u32 - 1
        } else {
            15 + (1 << self.linbits) - 1
        }
    }

    /// Raw table data `(codes, size, linbits)` for the test decoder.
    #[cfg(test)]
    pub(crate) fn raw(&self) -> (&'static [(u8, u32)], usize, u32) {
        (self.codes, self.size, self.linbits)
    }

    fn entry(&self, x: u32, y: u32) -> (u8, u32) {
        self.codes[x.min(15) as usize * self.size + y.min(15) as usize]
    }

    /// Bits to code the pair (x, y): Huffman code, any linbits, sign bits.
    pub fn pair_bits(&self, x: i32, y: i32) -> u32 {
        let (ax, ay) = (x.unsigned_abs(), y.unsigned_abs());
        debug_assert!(ax <= self.max_value() && ay <= self.max_value());
        let esc = |v: u32| if v >= 15 { self.linbits } else { 0 };
        u32::from(self.entry(ax, ay).0)
            + esc(ax)
            + esc(ay)
            + u32::from(ax != 0)
            + u32::from(ay != 0)
    }

    /// Write one pair in the order of `huffmancodebits()` (13818-3 §2.5.1):
    /// hcod, linbitsx, signx, linbitsy, signy.
    pub fn write_pair(&self, w: &mut BitWriter, x: i32, y: i32) {
        let (ax, ay) = (x.unsigned_abs(), y.unsigned_abs());
        let (len, code) = self.entry(ax, ay);
        w.put_bits(code, len.into());
        for (v, a) in [(x, ax), (y, ay)] {
            if self.linbits > 0 && a >= 15 {
                w.put_bits(a - 15, self.linbits);
            }
            if v != 0 {
                w.put_bit(v < 0);
            }
        }
    }
}

pub(crate) fn quad_table(table_b: bool) -> &'static [(u8, u32); 16] {
    if table_b {
        &tables::QUAD_B
    } else {
        &tables::QUAD_A
    }
}

/// Index into the quadruple tables: the printed `Value` column is the bits
/// v, w, x, y with v the most significant.
fn quad_index(q: &[i32]) -> usize {
    q.iter().fold(0, |acc, &v| (acc << 1) | usize::from(v != 0))
}

/// Bits to code one count1 quadruple (code plus one sign bit per non-zero).
fn quad_bits(table_b: bool, q: &[i32]) -> u32 {
    u32::from(quad_table(table_b)[quad_index(q)].0) + q.iter().filter(|&&v| v != 0).count() as u32
}

/// Everything the side information needs to describe a granule's Huffman
/// data (long blocks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HuffmanPlan {
    pub big_values: u16,
    pub table_select: [u8; 3],
    pub region0_count: u8,
    pub region1_count: u8,
    pub count1_table_b: bool,
    /// Number of count1 quadruples (implicit in the bitstream).
    pub count1: u16,
    /// Total Huffman bits (part3 of `part2_3_length`).
    pub bits: u32,
}

/// Region partition boundaries (13818-3 §2.5.2, `big_values` semantics):
/// returns `(big_values, count1)`. Starting from the top, pairs of zeros
/// (rzero) are skipped, then quadruples with all magnitudes ≤ 1 (count1);
/// the rest, in pairs, is the big_values region.
fn partition(ix: &[i32; GRANULE_LINES]) -> (usize, usize) {
    let mut n = GRANULE_LINES;
    while n >= 2 && ix[n - 1] == 0 && ix[n - 2] == 0 {
        n -= 2;
    }
    let rzero_start = n;
    while n >= 4 && ix[n - 4..n].iter().all(|v| v.abs() <= 1) {
        n -= 4;
    }
    (n / 2, (rzero_start - n) / 4)
}

/// Cheapest table for a run of pairs, given its maximum magnitude and
/// per-table prefix sums of pair bits. Returns `(table, bits)`.
///
/// original: tries every table able to code the maximum, as Annex C.1.5.4
/// §4.7 suggests ("additional coding gain can be achieved ... by trying all
/// of these tables").
fn best_table(
    max: u32,
    pairs: std::ops::Range<usize>,
    prefix: &[(usize, PairTable, Vec<u32>)],
) -> (u8, u32) {
    if pairs.is_empty() || max == 0 {
        return (0, 0);
    }
    prefix
        .iter()
        .filter(|(_, t, _)| t.max_value() >= max)
        .map(|(i, _, sums)| (*i as u8, sums[pairs.end] - sums[pairs.start]))
        .min_by_key(|&(_, bits)| bits)
        .expect("tables 23 and 31 code up to 8206 >= MAX_BIG_VALUE")
}

/// Choose the partition, region split and tables for a quantized granule
/// that minimise the Huffman bit count. `ix` magnitudes must be ≤
/// [`MAX_BIG_VALUE`].
///
/// original: the region split is "up to the implementation" (Annex C.1.5.4
/// §4.6). We search every `region0_count` (0..=15) and `region1_count`
/// (0..=7), using per-table prefix sums so each candidate costs O(tables).
pub fn plan(ix: &[i32; GRANULE_LINES], bands: &ScalefactorBands) -> HuffmanPlan {
    let (big_values, count1) = partition(ix);
    let bv_end = 2 * big_values;

    // Prefix sums of pair bits for every usable table, over the big_values
    // pairs. Entries for tables that cannot code some pair are never read
    // (best_table filters by the region maximum first), so saturate them.
    let prefix: Vec<(usize, PairTable, Vec<u32>)> = PAIR_TABLES
        .iter()
        .map(|&i| {
            let t = PairTable::get(i).expect("listed tables exist");
            let mut sums = Vec::with_capacity(big_values + 1);
            let mut acc = 0u32;
            sums.push(0);
            for p in ix[..bv_end].chunks_exact(2) {
                let fits = p.iter().all(|v| v.unsigned_abs() <= t.max_value());
                acc = acc.saturating_add(if fits {
                    t.pair_bits(p[0], p[1])
                } else {
                    1 << 24
                });
                sums.push(acc);
            }
            (i, t, sums)
        })
        .collect();

    // Maximum magnitude per scalefactor band, clipped to the big_values end.
    let bounds = bands.region_bounds();
    let band_max: Vec<u32> = bounds
        .windows(2)
        .map(|b| {
            let (lo, hi) = (b[0].min(bv_end), b[1].min(bv_end));
            ix[lo..hi]
                .iter()
                .map(|v| v.unsigned_abs())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let max_over = |from: usize, to: usize| band_max[from..to].iter().copied().max().unwrap_or(0);

    let mut best: Option<HuffmanPlan> = None;
    for r0 in 0..=15usize {
        for r1 in 0..=7usize {
            // Region 1 starts at band r0 + 1, region 2 at band r0 + r1 + 2
            // (13818-3 §2.5.2: each count is one less than its band count).
            let b1 = (r0 + 1).min(bounds.len() - 1);
            let b2 = (r0 + r1 + 2).min(bounds.len() - 1);
            let (s1, s2) = (bounds[b1].min(bv_end), bounds[b2].min(bv_end));
            let regions = [
                (0, s1, 0, b1),
                (s1, s2, b1, b2),
                (s2, bv_end, b2, band_max.len()),
            ];
            let mut table_select = [0u8; 3];
            let mut bits = 0;
            for (k, &(lo, hi, band_lo, band_hi)) in regions.iter().enumerate() {
                let (t, b) = best_table(max_over(band_lo, band_hi), lo / 2..hi / 2, &prefix);
                table_select[k] = t;
                bits += b;
            }
            if best.is_none_or(|p| bits < p.bits) {
                best = Some(HuffmanPlan {
                    big_values: big_values as u16,
                    table_select,
                    region0_count: r0 as u8,
                    region1_count: r1 as u8,
                    count1_table_b: false,
                    count1: count1 as u16,
                    bits,
                });
            }
            // Once region 2 is empty, larger region1_count changes nothing.
            if s2 >= bv_end {
                break;
            }
        }
    }
    let mut plan = best.expect("at least one candidate");

    // count1 region: pick table A or B, whichever is shorter (Annex C.1.5.4
    // §4.5).
    let quads = &ix[bv_end..bv_end + 4 * count1];
    let bits_a: u32 = quads.chunks_exact(4).map(|q| quad_bits(false, q)).sum();
    let bits_b: u32 = quads.chunks_exact(4).map(|q| quad_bits(true, q)).sum();
    plan.count1_table_b = bits_b < bits_a;
    plan.bits += bits_a.min(bits_b);
    plan
}

/// Write the Huffman data for a granule (13818-3 §2.5.1 `huffmancodebits()`,
/// long blocks: increasing frequency). Writes exactly `plan.bits` bits.
pub fn write(
    w: &mut BitWriter,
    ix: &[i32; GRANULE_LINES],
    plan: &HuffmanPlan,
    bands: &ScalefactorBands,
) {
    let bounds = bands.region_bounds();
    let bv_end = 2 * usize::from(plan.big_values);
    let last = bounds.len() - 1;
    let s1 = bounds[(usize::from(plan.region0_count) + 1).min(last)].min(bv_end);
    let s2 = bounds
        [(usize::from(plan.region0_count) + usize::from(plan.region1_count) + 2).min(last)]
    .min(bv_end);
    for (k, (lo, hi)) in [(0, s1), (s1, s2), (s2, bv_end)].into_iter().enumerate() {
        if lo == hi {
            continue;
        }
        let table = plan.table_select[k];
        if table == 0 {
            debug_assert!(ix[lo..hi].iter().all(|&v| v == 0));
            continue;
        }
        let t = PairTable::get(table.into()).expect("valid table_select");
        for p in ix[lo..hi].chunks_exact(2) {
            t.write_pair(w, p[0], p[1]);
        }
    }
    let table = quad_table(plan.count1_table_b);
    for q in ix[bv_end..bv_end + 4 * usize::from(plan.count1)].chunks_exact(4) {
        let (len, code) = table[quad_index(q)];
        w.put_bits(code, len.into());
        for &v in q {
            if v != 0 {
                w.put_bit(v < 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All codes of a table as (length, code) pairs.
    fn check_prefix_code(name: &str, codes: &[(u8, u32)]) {
        let mut kraft = 0.0f64;
        for (i, &(li, ci)) in codes.iter().enumerate() {
            assert!(li > 0 && li <= 19, "{name}[{i}] length {li}");
            assert!(ci >> li == 0, "{name}[{i}] code wider than hlen");
            kraft += 2f64.powi(-i32::from(li));
            for (j, &(lj, cj)) in codes.iter().enumerate() {
                if i != j && lj >= li {
                    assert_ne!(cj >> (lj - li), ci, "{name}: code {i} is a prefix of {j}");
                }
            }
        }
        assert!((kraft - 1.0).abs() < 1e-12, "{name}: Kraft sum {kraft}");
    }

    #[test]
    fn tables_are_complete_prefix_codes() {
        check_prefix_code("A", &tables::QUAD_A);
        check_prefix_code("B", &tables::QUAD_B);
        for i in [1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15, 16, 24] {
            let t = PairTable::get(i).unwrap();
            assert_eq!(t.size * t.size, t.codes.len());
            check_prefix_code(&format!("table {i}"), t.codes);
        }
        assert!(PairTable::get(0).is_none());
        assert!(PairTable::get(4).is_none());
        assert!(PairTable::get(14).is_none());
    }

    #[test]
    fn spot_values_from_table_b7() {
        // Table 1: (0,0) -> "1", (1,1) -> "000". Table 6: (3,3) -> "0000000".
        let t1 = PairTable::get(1).unwrap();
        assert_eq!(t1.entry(0, 0), (1, 0b1));
        assert_eq!(t1.entry(1, 1), (3, 0b000));
        assert_eq!(PairTable::get(6).unwrap().entry(3, 3), (7, 0));
        // Table 24 (ESC, linbits 4): (0,0) -> "1111".
        let t24 = PairTable::get(24).unwrap();
        assert_eq!(t24.entry(0, 0), (4, 0b1111));
        assert_eq!(t24.max_value(), 30);
        assert_eq!(PairTable::get(23).unwrap().max_value(), 8206);
        // Quad A: 0000 -> "1", 0011 (x = y = 1) -> "00101".
        assert_eq!(tables::QUAD_A[0], (1, 1));
        assert_eq!(tables::QUAD_A[0b0011], (5, 0b00101));
    }

    #[test]
    fn partition_regions() {
        let mut ix = [0i32; GRANULE_LINES];
        ix[0] = 5;
        ix[1] = -2;
        ix[10] = 1;
        ix[13] = -1;
        // Top non-zero at 13 -> rzero pairs from line 14. Quads are counted
        // down from there: 10..14, 6..10 and 2..6 are within ±1; fewer than 4
        // lines remain, so big_values is the single pair 0..2.
        assert_eq!(partition(&ix), (1, 3));
        assert_eq!(partition(&[0; GRANULE_LINES]), (0, 0));
    }

    /// Encode a spectrum, decode it with the reference decoder (which sees
    /// only the side-info fields and the bits), and compare.
    fn round_trip(ix: &[i32; GRANULE_LINES], rate: u32) {
        use crate::refdec::{BitReader, HuffmanSideInfo, decode_huffman};
        let bands = ScalefactorBands::long(rate).unwrap();
        let p = plan(ix, &bands);
        let mut w = BitWriter::new();
        write(&mut w, ix, &p, &bands);
        assert_eq!(
            w.bit_len(),
            p.bits as usize,
            "plan.bits must match written bits"
        );
        let bytes = w.into_bytes();
        let si = HuffmanSideInfo {
            big_values: p.big_values.into(),
            table_select: p.table_select.map(usize::from),
            region0_count: p.region0_count.into(),
            region1_count: p.region1_count.into(),
            count1_table_b: p.count1_table_b,
            part3_length: p.bits as usize,
        };
        let decoded = decode_huffman(
            &mut BitReader::new(&bytes),
            &si,
            bands.region_bounds(),
            |t| PairTable::get(t).unwrap().raw(),
            quad_table(si.count1_table_b),
        );
        assert_eq!(&decoded[..], &ix[..]);
    }

    #[test]
    fn huffman_round_trip() {
        // original: 64-bit LCG (Knuth MMIX constants).
        let mut seed = 5u64;
        let mut rand = |n: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        for case in 0..60 {
            let rate = [44_100, 48_000, 32_000, 22_050, 24_000][case % 5];
            let mut ix = [0i32; GRANULE_LINES];
            // Spectra shaped like music: large at low frequencies, decaying,
            // with a zero tail of varying length. Some cases use escapes up
            // to MAX_BIG_VALUE.
            let peak = [1u64, 2, 4, 15, 16, 40, 300, 8192][case % 8];
            let len = 50 + rand(527) as usize;
            for (i, v) in ix[..len].iter_mut().enumerate() {
                let scale = (peak * 40 / (40 + i as u64)).max(1);
                let mag = rand(scale + 1).min(u64::from(MAX_BIG_VALUE)) as i32;
                *v = if rand(2) == 0 { mag } else { -mag };
            }
            round_trip(&ix, rate);
        }
        // Edge cases: silence, all-count1, a single max value at the top.
        round_trip(&[0; GRANULE_LINES], 44_100);
        round_trip(&[1; GRANULE_LINES], 44_100);
        let mut ix = [0; GRANULE_LINES];
        ix[575] = -(MAX_BIG_VALUE as i32);
        round_trip(&ix, 48_000);
    }
}
