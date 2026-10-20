//! Randomness for the kernel: the ChaCha20 stream cipher of RFC 8439 and a
//! cryptographically secure random number generator ([`Csprng`]) built on
//! it, behind `/dev/random`, `/dev/urandom` and `getrandom(2)`.
//!
//! # ChaCha20
//!
//! [`block`] is the ChaCha20 block function exactly as RFC 8439 section
//! 2.3 specifies it: a 4x4 state of 32-bit words (the constants
//! "expand 32-byte k", the 256-bit key, a 32-bit block counter and a 96-bit
//! nonce, all little-endian), 20 rounds (10 column rounds interleaved with
//! 10 diagonal rounds of the quarter round of section 2.1), the input state
//! added back and the result serialized little-endian. [`xor_keystream`] is
//! the cipher of section 2.4. Both are checked against the RFC's test
//! vectors.
//!
//! # The generator
//!
//! [`Csprng`] keeps a 256-bit key and nothing else. It uses ChaCha20 the
//! way D. J. Bernstein's "fast-key-erasure random-number generators"
//! (2017) describes:
//!
//! - **Output** ([`Csprng::fill`]): the keystream of the current key (nonce
//!   0, counter from 0) is computed; its first 32 bytes become the next key
//!   and the rest is handed out. The old key is gone before the caller sees
//!   a byte, so a later compromise of the state reveals nothing about
//!   earlier output (backtracking resistance). At most [`MAX_PER_KEY`]
//!   bytes come from one key; longer requests rekey on the way.
//! - **Input** ([`Csprng::mix`]): new material is folded in 32 bytes at a
//!   time as `key = F(key) XOR chunk`, where `F(key)` is the first half of
//!   a ChaCha20 block under the current key with a nonce reserved for
//!   mixing (so it never equals output), followed by one more step over
//!   the input's length. Because `F(key)` is pseudorandom and unknown to
//!   anyone who does not know the key, XORing in any value chosen without
//!   that knowledge (an attacker's write to `/dev/urandom`, say) leaves the
//!   key exactly as unpredictable as before, while real entropy adds to it.
//! - **Readiness**: the generator counts the bits of entropy the caller
//!   credits ([`Csprng::add_entropy`]); it is [`ready`](Csprng::ready) once
//!   256 bits have been credited. Material mixed without credit (user
//!   writes, as random(4) says: "this will not result in a higher entropy
//!   count") changes the output but not the count.
//!
//! The kernel seeds it from RDSEED / RDRAND and the jitter of the
//! timestamp counter at boot and reseeds it from time to time; this crate
//! only does the arithmetic, so it is tested on the host.

#![no_std]

/// The four constant words of the ChaCha state: "expand 32-byte k" read as
/// little-endian 32-bit words (RFC 8439 section 2.3).
pub const CONSTANTS: [u32; 4] = [0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574];

/// Bytes in one ChaCha20 block.
pub const BLOCK_LEN: usize = 64;
/// Bytes in a ChaCha20 key.
pub const KEY_LEN: usize = 32;
/// Bytes in a ChaCha20 nonce (RFC 8439's 96-bit nonce).
pub const NONCE_LEN: usize = 12;

/// The ChaCha quarter round on four words (RFC 8439 section 2.1).
pub fn quarter_round(a: u32, b: u32, c: u32, d: u32) -> (u32, u32, u32, u32) {
    let mut s = [a, b, c, d];
    qr(&mut s, 0, 1, 2, 3);
    (s[0], s[1], s[2], s[3])
}

/// The quarter round on four words of a state (`QUARTERROUND(x, y, z, w)`,
/// RFC 8439 section 2.2).
#[inline(always)]
fn qr(s: &mut [u32], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(7);
}

/// The ChaCha20 state for `key`, block `counter` and `nonce` (RFC 8439
/// section 2.3): constants, key, counter, nonce, each little-endian.
pub fn initial_state(key: &[u8; KEY_LEN], counter: u32, nonce: &[u8; NONCE_LEN]) -> [u32; 16] {
    let mut s = [0u32; 16];
    s[..4].copy_from_slice(&CONSTANTS);
    for (i, word) in key.chunks_exact(4).enumerate() {
        s[4 + i] = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
    }
    s[12] = counter;
    for (i, word) in nonce.chunks_exact(4).enumerate() {
        s[13 + i] = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
    }
    s
}

/// The 20 rounds and the final addition, on a state.
fn rounds(input: &[u32; 16]) -> [u32; 16] {
    let mut s = *input;
    for _ in 0..10 {
        // A column round...
        qr(&mut s, 0, 4, 8, 12);
        qr(&mut s, 1, 5, 9, 13);
        qr(&mut s, 2, 6, 10, 14);
        qr(&mut s, 3, 7, 11, 15);
        // ...and a diagonal round.
        qr(&mut s, 0, 5, 10, 15);
        qr(&mut s, 1, 6, 11, 12);
        qr(&mut s, 2, 7, 8, 13);
        qr(&mut s, 3, 4, 9, 14);
    }
    for (w, i) in s.iter_mut().zip(input) {
        *w = w.wrapping_add(*i);
    }
    s
}

/// The ChaCha20 block function (RFC 8439 section 2.3): 64 bytes of
/// keystream for `key`, block `counter` and `nonce`.
pub fn block(key: &[u8; KEY_LEN], counter: u32, nonce: &[u8; NONCE_LEN]) -> [u8; BLOCK_LEN] {
    let mut state = initial_state(key, counter, nonce);
    let out = rounds(&state);
    let mut bytes = [0u8; BLOCK_LEN];
    for (chunk, w) in bytes.chunks_exact_mut(4).zip(out) {
        chunk.copy_from_slice(&w.to_le_bytes());
    }
    wipe_words(&mut state);
    bytes
}

/// The ChaCha20 cipher (RFC 8439 section 2.4): XORs `data` with the
/// keystream that starts at block `counter`. Encryption and decryption are
/// the same operation. The counter must not wrap within `data` (at most
/// 256 GiB from counter 0); this panics if it would.
pub fn xor_keystream(key: &[u8; KEY_LEN], counter: u32, nonce: &[u8; NONCE_LEN], data: &mut [u8]) {
    for (i, chunk) in data.chunks_mut(BLOCK_LEN).enumerate() {
        let n = u32::try_from(i).ok().and_then(|i| counter.checked_add(i)).expect("ChaCha20 block counter overflow");
        let mut ks = block(key, n, nonce);
        for (d, k) in chunk.iter_mut().zip(ks.iter()) {
            *d ^= k;
        }
        wipe(&mut ks);
    }
}

/// Overwrite secret bytes with zeros in a way the compiler keeps.
pub fn wipe(bytes: &mut [u8]) {
    for b in bytes.iter_mut() {
        // SAFETY: `b` is a valid, aligned, exclusive reference to a byte.
        unsafe { core::ptr::write_volatile(b, 0) };
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}

fn wipe_words(words: &mut [u32]) {
    for w in words.iter_mut() {
        // SAFETY: `w` is a valid, aligned, exclusive reference to a word.
        unsafe { core::ptr::write_volatile(w, 0) };
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}

/// The most output bytes one key produces before the generator rekeys:
/// 64 blocks less the 32 bytes that become the next key.
pub const MAX_PER_KEY: usize = 64 * BLOCK_LEN - KEY_LEN;

/// Bits of credited entropy after which the generator counts as seeded.
pub const READY_BITS: u32 = 256;

/// The nonce of the keystream that output and rekeying use.
const OUTPUT_NONCE: [u8; NONCE_LEN] = [0; NONCE_LEN];
/// The nonce of the blocks that mixing uses: never the output nonce, so no
/// mixing step ever reuses keystream that was handed out.
const MIX_NONCE: [u8; NONCE_LEN] = *b"poly-mix\0\0\0\x01";

/// A fast-key-erasure random number generator on ChaCha20 (see the crate
/// documentation).
pub struct Csprng {
    key: [u8; KEY_LEN],
    /// Credited entropy in bits, saturating at [`READY_BITS`].
    credited: u32,
    /// Times the key was replaced by output or input (for tests and logs).
    generation: u64,
}

impl core::fmt::Debug for Csprng {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Never the key.
        f.debug_struct("Csprng")
            .field("credited", &self.credited)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl Default for Csprng {
    fn default() -> Self {
        Csprng::new()
    }
}

impl Csprng {
    /// A generator with an all-zero key and no credited entropy: its output
    /// is predictable until something is mixed in.
    pub const fn new() -> Csprng {
        Csprng { key: [0; KEY_LEN], credited: 0, generation: 0 }
    }

    /// Whether at least [`READY_BITS`] bits of entropy have been credited.
    pub fn ready(&self) -> bool {
        self.credited >= READY_BITS
    }

    /// Credited entropy in bits (at most [`READY_BITS`]).
    pub fn credited_bits(&self) -> u32 {
        self.credited
    }

    /// How many times the key has changed.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Fold `data` into the key without crediting any entropy.
    pub fn mix(&mut self, data: &[u8]) {
        for chunk in data.chunks(KEY_LEN) {
            let mut padded = [0u8; KEY_LEN];
            padded[..chunk.len()].copy_from_slice(chunk);
            self.mix_step(&padded);
            wipe(&mut padded);
        }
        // The length, so that inputs differing only in trailing zeros mix
        // differently.
        let mut len = [0u8; KEY_LEN];
        len[..8].copy_from_slice(&(data.len() as u64).to_le_bytes());
        len[8] = 0xff;
        self.mix_step(&len);
    }

    /// Fold `data` in and credit `bits` of entropy to it.
    pub fn add_entropy(&mut self, data: &[u8], bits: u32) {
        self.mix(data);
        self.credited = self.credited.saturating_add(bits).min(READY_BITS);
    }

    fn mix_step(&mut self, chunk: &[u8; KEY_LEN]) {
        let mut f = block(&self.key, 0, &MIX_NONCE);
        for ((k, a), b) in self.key.iter_mut().zip(&f[..KEY_LEN]).zip(chunk) {
            *k = a ^ b;
        }
        wipe(&mut f);
        self.generation += 1;
    }

    /// Fill `out` with random bytes. The key is replaced (from the first
    /// half of block 0 of its own keystream) before any byte is returned,
    /// and again every [`MAX_PER_KEY`] bytes.
    pub fn fill(&mut self, out: &mut [u8]) {
        for piece in out.chunks_mut(MAX_PER_KEY) {
            let mut old = self.key;
            let mut first = block(&old, 0, &OUTPUT_NONCE);
            self.key.copy_from_slice(&first[..KEY_LEN]);
            self.generation += 1;
            let n = piece.len().min(BLOCK_LEN - KEY_LEN);
            piece[..n].copy_from_slice(&first[KEY_LEN..KEY_LEN + n]);
            wipe(&mut first);
            for (i, chunk) in piece[n..].chunks_mut(BLOCK_LEN).enumerate() {
                // At most 63 more blocks: MAX_PER_KEY bounds the piece.
                let mut b = block(&old, i as u32 + 1, &OUTPUT_NONCE);
                chunk.copy_from_slice(&b[..chunk.len()]);
                wipe(&mut b);
            }
            wipe(&mut old);
        }
    }
}

#[cfg(test)]
mod tests;
