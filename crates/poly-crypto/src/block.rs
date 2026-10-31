//! The message buffering and Merkle–Damgård padding shared by MD5 (RFC
//! 1321 3.1–3.2) and SHA-1/SHA-2 (FIPS 180-4 5.1): a `1` bit, zeros, and the
//! message length in bits in the last `len_field` bytes of the final block.

/// Buffered input for a hash with `N`-byte blocks.
#[derive(Clone, Debug)]
pub(crate) struct BlockBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
    /// Bytes absorbed so far.
    total: u128,
}

impl<const N: usize> BlockBuf<N> {
    pub(crate) const fn new() -> Self {
        BlockBuf { buf: [0; N], len: 0, total: 0 }
    }

    /// The message length so far in bits (modulo 2^128).
    pub(crate) fn bit_len(&self) -> u128 {
        self.total.wrapping_mul(8)
    }

    /// Absorb `data`, calling `f` for every complete block.
    pub(crate) fn update(&mut self, mut data: &[u8], mut f: impl FnMut(&[u8; N])) {
        self.total = self.total.wrapping_add(data.len() as u128);
        if self.len > 0 {
            let n = (N - self.len).min(data.len());
            self.buf[self.len..self.len + n].copy_from_slice(&data[..n]);
            self.len += n;
            data = &data[n..];
            if self.len < N {
                return;
            }
            f(&self.buf);
            self.len = 0;
        }
        let mut chunks = data.chunks_exact(N);
        for c in &mut chunks {
            let mut block = [0u8; N];
            block.copy_from_slice(c);
            f(&block);
        }
        let rest = chunks.remainder();
        self.buf[..rest.len()].copy_from_slice(rest);
        self.len = rest.len();
    }

    /// Append the padding and `len_field` (the encoded bit length), calling
    /// `f` for the final block or blocks.
    pub(crate) fn pad(&mut self, len_field: &[u8], mut f: impl FnMut(&[u8; N])) {
        let mut block = self.buf;
        block[self.len] = 0x80;
        block[self.len + 1..].fill(0);
        if self.len + 1 > N - len_field.len() {
            f(&block);
            block = [0; N];
        }
        block[N - len_field.len()..].copy_from_slice(len_field);
        f(&block);
        self.len = 0;
    }
}
