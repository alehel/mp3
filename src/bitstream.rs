//! MSB-first bit writer used for frame headers, side information and main data.
//!
//! Original work. MPEG audio bitstreams are big-endian at the bit level
//! (ISO/IEC 11172-3 §2.1, "bslbf"/"uimsbf" conventions).

/// Accumulates bits most-significant-bit first into a byte vector.
#[derive(Debug, Default, Clone)]
pub struct BitWriter {
    bytes: Vec<u8>,
    /// Bits not yet flushed to `bytes`, left-aligned in the low `nbits` bits.
    acc: u64,
    nbits: u32,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(bytes),
            ..Self::default()
        }
    }

    /// Write the low `n` bits of `value`, MSB first. `n` must be ≤ 32.
    ///
    /// # Panics
    /// In debug builds, panics if `value` has bits set above `n`.
    pub fn put_bits(&mut self, value: u32, n: u32) {
        assert!(n <= 32, "put_bits supports at most 32 bits at a time");
        if n == 0 {
            return;
        }
        debug_assert!(
            n == 32 || value >> n == 0,
            "value {value:#x} does not fit in {n} bits"
        );
        self.acc = (self.acc << n) | u64::from(value);
        self.nbits += n;
        while self.nbits >= 8 {
            self.nbits -= 8;
            self.bytes.push((self.acc >> self.nbits) as u8);
        }
        // Keep only the unflushed bits so `acc` never overflows.
        self.acc &= (1u64 << self.nbits) - 1;
    }

    /// Write a single bit.
    pub fn put_bit(&mut self, bit: bool) {
        self.put_bits(u32::from(bit), 1);
    }

    /// Total number of bits written so far.
    pub fn bit_len(&self) -> usize {
        self.bytes.len() * 8 + self.nbits as usize
    }

    /// Pad with zero bits up to the next byte boundary.
    pub fn byte_align(&mut self) {
        let rem = self.nbits % 8;
        if rem != 0 {
            self.put_bits(0, 8 - rem);
        }
    }

    /// Append whole bytes. The writer must be byte-aligned.
    pub fn put_bytes(&mut self, data: &[u8]) {
        assert_eq!(self.nbits, 0, "put_bytes requires byte alignment");
        self.bytes.extend_from_slice(data);
    }

    /// Finish, zero-padding to a byte boundary, and return the bytes.
    pub fn into_bytes(mut self) -> Vec<u8> {
        self.byte_align();
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_msb_first() {
        let mut w = BitWriter::new();
        w.put_bits(0b101, 3);
        w.put_bits(0b11110, 5);
        assert_eq!(w.into_bytes(), vec![0b1011_1110]);
    }

    #[test]
    fn spans_bytes_and_pads() {
        let mut w = BitWriter::new();
        w.put_bits(0x7FF, 11); // MPEG sync word
        w.put_bits(0b1, 1);
        assert_eq!(w.bit_len(), 12);
        assert_eq!(w.into_bytes(), vec![0xFF, 0xF0]);
    }

    #[test]
    fn handles_32_bit_values() {
        let mut w = BitWriter::new();
        w.put_bit(true);
        w.put_bits(0xDEAD_BEEF, 32);
        w.byte_align();
        assert_eq!(w.bit_len(), 40);
        assert_eq!(w.into_bytes(), vec![0xEF, 0x56, 0xDF, 0x77, 0x80]);
    }

    #[test]
    fn zero_bits_is_noop() {
        let mut w = BitWriter::new();
        w.put_bits(0, 0);
        assert_eq!(w.bit_len(), 0);
        assert!(w.into_bytes().is_empty());
    }
}
