//! Structures with pointers laid out in one block, the way CryptoAPI hands
//! them out (a `CERT_INFO` and everything it points to in one allocation;
//! `CryptDecodeObjectEx`'s output in the caller's buffer). The block is
//! built with offsets and turned into bytes for the address it will live
//! at (as `poly-security` lays out token information).

use alloc::vec::Vec;

/// A block under construction: the bytes and the places that hold
/// pointers (stored as offsets until [`Flat::finish`]).
#[derive(Clone, Debug, Default)]
pub struct Flat {
    buf: Vec<u8>,
    fixups: Vec<usize>,
}

impl Flat {
    /// A block that starts with `fixed` zero bytes (the top structure).
    pub fn new(fixed: usize) -> Flat {
        Flat { buf: alloc::vec![0; fixed], fixups: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    fn align(&mut self) {
        while !self.buf.len().is_multiple_of(8) {
            self.buf.push(0);
        }
    }

    /// Append `n` zero bytes (8-aligned); their offset.
    pub fn reserve(&mut self, n: usize) -> usize {
        self.align();
        let at = self.buf.len();
        self.buf.resize(at + n, 0);
        at
    }

    /// Append `data` (8-aligned); its offset.
    pub fn data(&mut self, data: &[u8]) -> usize {
        let at = self.reserve(data.len());
        self.buf[at..at + data.len()].copy_from_slice(data);
        at
    }

    pub fn put_u32(&mut self, at: usize, v: u32) {
        self.buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    pub fn put_u64(&mut self, at: usize, v: u64) {
        self.buf[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }

    /// A pointer at `at` to offset `target` of the block.
    pub fn put_ptr(&mut self, at: usize, target: usize) {
        self.put_u64(at, target as u64);
        self.fixups.push(at);
    }

    /// A pointer at `at` to a copy of `data` (NULL for empty data).
    pub fn put_data_ptr(&mut self, at: usize, data: &[u8]) {
        if !data.is_empty() {
            let t = self.data(data);
            self.put_ptr(at, t);
        }
    }

    /// A `CRYPT_DATA_BLOB`-shaped pair at `at`: `cbData` (DWORD), then
    /// `pbData` at `at + 8`.
    pub fn put_blob(&mut self, at: usize, data: &[u8]) {
        self.put_u32(at, data.len() as u32);
        self.put_data_ptr(at + 8, data);
    }

    /// A `CRYPT_BIT_BLOB` at `at`: `cbData`, `pbData`, `cUnusedBits` (+16).
    pub fn put_bit_blob(&mut self, at: usize, unused: u8, data: &[u8]) {
        self.put_blob(at, data);
        self.put_u32(at + 16, u32::from(unused));
    }

    /// A pointer at `at` to a NUL-terminated copy of an ASCII string.
    pub fn put_cstr(&mut self, at: usize, s: &str) {
        let mut v = Vec::from(s.as_bytes());
        v.push(0);
        let t = self.data(&v);
        self.put_ptr(at, t);
    }

    /// A pointer at `at` to a NUL-terminated UTF-16 copy of `s`; returns
    /// its length in characters.
    pub fn put_wstr(&mut self, at: usize, s: &str) -> usize {
        let w: Vec<u8> = s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
        let t = self.data(&w);
        self.put_ptr(at, t);
        w.len() / 2 - 1
    }

    /// The bytes for a block that will live at `base`.
    pub fn finish(mut self, base: usize) -> Vec<u8> {
        for at in core::mem::take(&mut self.fixups) {
            let off = u64::from_le_bytes(self.buf[at..at + 8].try_into().unwrap_or([0; 8]));
            self.put_u64(at, off + base as u64);
        }
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointers_are_relocated() {
        let mut f = Flat::new(24);
        f.put_blob(0, b"abc");
        f.put_cstr(16, "1.2");
        let out = f.finish(0x1000);
        assert_eq!(u32::from_le_bytes(out[0..4].try_into().unwrap()), 3);
        let p = u64::from_le_bytes(out[8..16].try_into().unwrap()) as usize;
        assert_eq!(&out[p - 0x1000..p - 0x1000 + 3], b"abc");
        let s = u64::from_le_bytes(out[16..24].try_into().unwrap()) as usize - 0x1000;
        assert_eq!(&out[s..s + 4], b"1.2\0");
        let mut e = Flat::new(16);
        e.put_blob(0, b"");
        assert_eq!(e.finish(0x1000), [0; 16]);
    }
}
