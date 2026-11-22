//! Clean-room `[MS-CAB]` / `[MS-PATCH]` LZX decompressor (`tcompTYPE_LZX`).
//!
//! Written strictly from Microsoft's public `[MS-CAB]` and `[MS-PATCH]` LZX
//! specifications. Supports window sizes from 32 KiB (`window_bits = 15`) up to
//! 2 MiB (`window_bits = 21`), all three LZX block types (verbatim, aligned
//! offset, and uncompressed), canonical Huffman tree decoding (pretree, main
//! tree, length tree, aligned-offset tree), repeated offset registers
//! (`R0`/`R1`/`R2`), Intel `0xE8` call translation post-processing across
//! 32 KiB frames, and multi-`CFDATA` folders where LZX blocks and sliding
//! window history span across chunk boundaries.

use crate::Error;
use alloc::vec;
use alloc::vec::Vec;

/// Minimum LZX window size exponent (32 KiB = `1 << 15`).
pub const MIN_WINDOW_BITS: u16 = 15;
/// Maximum LZX window size exponent (2 MiB = `1 << 21`).
pub const MAX_WINDOW_BITS: u16 = 21;
/// Maximum uncompressed size of a single `[MS-CAB]` LZX chunk (32 KiB).
pub const MAX_FRAME_SIZE: usize = 32_768;

const NUM_CHARS: usize = 256;
const NUM_LENGTH_ELEMENTS: usize = 249;
const NUM_ALIGNED_ELEMENTS: usize = 8;
const NUM_PRETREE_ELEMENTS: usize = 20;
const MAX_POSITION_SLOTS: usize = 50;
const MAX_CODE_LEN: usize = 16;

/// Fast primary lookup table width in bits (1024 entries = 4 KiB per tree).
const FAST_BITS: u32 = 10;
const FAST_SIZE: usize = 1 << FAST_BITS;

/// Intel E8 translation is disabled after 32,768 frames (1 GiB uncompressed).
const MAX_E8_FRAMES: u32 = 32_768;
const MAX_E8_OFFSET: u64 = 0x4000_0000;

/// Extra footer bits per position slot (`[MS-PATCH]` LZX position slot table).
const EXTRA_BITS: [u8; MAX_POSITION_SLOTS] = {
    let mut table = [0u8; MAX_POSITION_SLOTS];
    let mut i = 0;
    while i < MAX_POSITION_SLOTS {
        table[i] = if i < 4 {
            0
        } else if i < 36 {
            ((i - 2) / 2) as u8
        } else {
            17
        };
        i += 1;
    }
    table
};

/// Base formatted offset per position slot (`[MS-PATCH]` LZX position slot table).
const BASE_POSITION: [u32; MAX_POSITION_SLOTS] = {
    let mut table = [0u32; MAX_POSITION_SLOTS];
    let mut base = 0u32;
    let mut i = 0;
    while i < MAX_POSITION_SLOTS {
        table[i] = base;
        base += 1u32 << EXTRA_BITS[i];
        i += 1;
    }
    table
};

/// Number of position slots for a given LZX window exponent (`15..=21`).
pub fn position_slots(window_bits: u16) -> Result<usize, Error> {
    match window_bits {
        15 => Ok(30),
        16 => Ok(32),
        17 => Ok(34),
        18 => Ok(36),
        19 => Ok(38),
        20 => Ok(42),
        21 => Ok(50),
        _ => Err(Error::Compression),
    }
}

/// `[MS-CAB]` / `[MS-PATCH]` LZX bitstream reader.
///
/// The compressed bitstream consists of 16-bit little-endian words whose bits
/// are read from most-significant bit to least-significant bit, except inside
/// uncompressed blocks which switch to a raw byte stream on a 16-bit boundary.
struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    /// Left-aligned bit buffer: the top `bit_count` bits are valid and all
    /// lower `64 - bit_count` bits are zero.
    bit_buf: u64,
    bit_count: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, byte_pos: 0, bit_buf: 0, bit_count: 0 }
    }

    #[inline]
    fn refill(&mut self) {
        while self.bit_count <= 48 && self.byte_pos + 2 <= self.data.len() {
            let word = u16::from_le_bytes([self.data[self.byte_pos], self.data[self.byte_pos + 1]]);
            self.byte_pos += 2;
            self.bit_buf |= u64::from(word) << (48 - self.bit_count);
            self.bit_count += 16;
        }
    }

    #[inline]
    fn read_bits(&mut self, n: u32) -> Result<u32, Error> {
        if n == 0 {
            return Ok(0);
        }
        debug_assert!(n <= 32);
        if self.bit_count < n {
            self.refill();
        }
        if self.bit_count < n {
            return Err(Error::Truncated);
        }
        let val = (self.bit_buf >> (64 - n)) as u32;
        self.bit_buf <<= n;
        self.bit_count -= n;
        Ok(val)
    }

    #[inline]
    fn peek_16(&mut self) -> u16 {
        if self.bit_count < 16 {
            self.refill();
        }
        (self.bit_buf >> 48) as u16
    }

    #[inline]
    fn consume(&mut self, n: u32) -> Result<(), Error> {
        if n > self.bit_count {
            return Err(Error::Truncated);
        }
        self.bit_buf <<= n;
        self.bit_count -= n;
        Ok(())
    }

    /// Consume 1..=16 bits of alignment padding after an uncompressed block
    /// header and transition to the raw byte stream on a 16-bit boundary.
    fn align_for_uncompressed(&mut self) -> Result<(), Error> {
        let rem = self.bit_count % 16;
        if rem == 0 {
            let _ = self.read_bits(16)?;
        } else {
            self.consume(rem)?;
        }
        let rewind_bytes = ((self.bit_count / 16) as usize) * 2;
        self.byte_pos = self.byte_pos.checked_sub(rewind_bytes).ok_or(Error::Corrupt)?;
        self.bit_buf = 0;
        self.bit_count = 0;
        Ok(())
    }

    fn read_raw_bytes(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.byte_pos.checked_add(n).ok_or(Error::Corrupt)?;
        let slice = self.data.get(self.byte_pos..end).ok_or(Error::Truncated)?;
        self.byte_pos = end;
        Ok(slice)
    }

    fn read_u32_le(&mut self) -> Result<u32, Error> {
        let b = self.read_raw_bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    #[inline]
    fn byte_pos(&self) -> usize {
        self.byte_pos
    }

    #[inline]
    fn remaining_bytes(&self) -> usize {
        self.data.len().saturating_sub(self.byte_pos)
    }
}

/// Canonical Huffman table with a 10-bit primary lookup table and exact
/// canonical fallback for 11..=16-bit codes.
#[derive(Clone, Debug)]
struct HuffmanTable {
    fast: Vec<u32>,
    max_code: [u32; MAX_CODE_LEN + 1],
    code_base: [u32; MAX_CODE_LEN + 1],
    sym_base: [u32; MAX_CODE_LEN + 1],
    symbols: Vec<u16>,
    empty: bool,
}

impl HuffmanTable {
    fn empty() -> Self {
        Self {
            fast: Vec::new(),
            max_code: [0; MAX_CODE_LEN + 1],
            code_base: [0; MAX_CODE_LEN + 1],
            sym_base: [0; MAX_CODE_LEN + 1],
            symbols: Vec::new(),
            empty: true,
        }
    }

    fn build(lengths: &[u8]) -> Result<Self, Error> {
        let mut bl_count = [0u32; MAX_CODE_LEN + 1];
        let mut total_syms = 0u32;
        for &len in lengths {
            if usize::from(len) > MAX_CODE_LEN {
                return Err(Error::Corrupt);
            }
            if len > 0 {
                bl_count[usize::from(len)] += 1;
                total_syms += 1;
            }
        }
        if total_syms == 0 {
            return Ok(Self::empty());
        }

        let mut code_base = [0u32; MAX_CODE_LEN + 1];
        let mut sym_base = [0u32; MAX_CODE_LEN + 1];
        let mut max_code = [0u32; MAX_CODE_LEN + 1];
        let mut code = 0u32;
        let mut sym_off = 0u32;
        for len in 1..=MAX_CODE_LEN {
            code <<= 1;
            code_base[len] = code;
            sym_base[len] = sym_off;
            let cnt = bl_count[len];
            let end = code + cnt;
            if end > (1u32 << len) {
                return Err(Error::Corrupt);
            }
            max_code[len] = end << (MAX_CODE_LEN - len);
            code = end;
            sym_off += cnt;
        }

        // Valid LZX canonical trees are either complete (Kraft sum == 1) or a
        // single-symbol tree of bit length 1.
        let is_complete = code == (1u32 << MAX_CODE_LEN);
        let is_single = total_syms == 1 && bl_count[1] == 1;
        if !is_complete && !is_single {
            return Err(Error::Corrupt);
        }

        let mut symbols = vec![0u16; total_syms as usize];
        let mut next_sym = sym_base;
        let mut fast = vec![0u32; FAST_SIZE];
        for (sym, &len) in lengths.iter().enumerate() {
            if len == 0 {
                continue;
            }
            let l = usize::from(len);
            let idx = next_sym[l] as usize;
            next_sym[l] += 1;
            symbols[idx] = sym as u16;
            if u32::from(len) <= FAST_BITS {
                let sym_code = code_base[l] + (idx as u32 - sym_base[l]);
                let fill_shift = FAST_BITS - u32::from(len);
                let start = (sym_code << fill_shift) as usize;
                let count = 1usize << fill_shift;
                let entry = (u32::from(len) << 16) | (sym as u32);
                fast[start..start + count].fill(entry);
            }
        }

        Ok(Self { fast, max_code, code_base, sym_base, symbols, empty: false })
    }

    #[inline]
    fn decode(&self, bits: &mut BitReader<'_>) -> Result<u16, Error> {
        if self.empty {
            return Err(Error::Corrupt);
        }
        let peek = bits.peek_16();
        let entry = self.fast[usize::from(peek >> (16 - FAST_BITS))];
        if entry != 0 {
            let len = entry >> 16;
            bits.consume(len)?;
            return Ok((entry & 0xFFFF) as u16);
        }
        let peek_u32 = u32::from(peek);
        for len in (FAST_BITS as usize + 1)..=MAX_CODE_LEN {
            if peek_u32 < self.max_code[len] {
                let code = peek_u32 >> (MAX_CODE_LEN - len);
                let idx = self.sym_base[len] + (code - self.code_base[len]);
                bits.consume(len as u32)?;
                return self.symbols.get(idx as usize).copied().ok_or(Error::Corrupt);
            }
        }
        Err(Error::Corrupt)
    }
}

/// Active LZX block type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlockType {
    None,
    Verbatim,
    Aligned,
    Uncompressed,
}

/// Reverse Intel `0xE8` call translation on a single decoded frame (`[MS-PATCH]`).
///
/// Must be applied to the output copy of the frame, never to the sliding
/// window itself, because subsequent matches reference the pre-translated bytes.
pub fn reverse_e8_translation(chunk: &mut [u8], chunk_offset: u64, filesize: u32) {
    let file_size = i64::from(filesize as i32);
    if file_size <= 0 || chunk_offset >= MAX_E8_OFFSET || chunk.len() <= 10 {
        return;
    }
    let limit = chunk.len() - 10;
    let mut i = 0usize;
    while i < limit {
        if chunk[i] == 0xE8 {
            let cur_pos = (chunk_offset + i as u64) as i64;
            let abs_val = i64::from(i32::from_le_bytes([chunk[i + 1], chunk[i + 2], chunk[i + 3], chunk[i + 4]]));
            if abs_val >= -cur_pos && abs_val < file_size {
                let rel_val = if abs_val >= 0 { abs_val - cur_pos } else { abs_val + file_size };
                chunk[i + 1..i + 5].copy_from_slice(&(rel_val as i32).to_le_bytes());
            }
            i += 5;
        } else {
            i += 1;
        }
    }
}

/// Read delta-encoded canonical Huffman path lengths for `lens[start..end]`
/// using a 20-element pretree (`[MS-PATCH]` "Encoding the Trees and Pretrees").
fn read_tree_lengths(bits: &mut BitReader<'_>, lens: &mut [u8], start: usize, end: usize) -> Result<(), Error> {
    let mut pre_lens = [0u8; NUM_PRETREE_ELEMENTS];
    for l in &mut pre_lens {
        *l = bits.read_bits(4)? as u8;
    }
    let pretree = HuffmanTable::build(&pre_lens)?;
    let mut i = start;
    while i < end {
        let sym = pretree.decode(bits)?;
        match sym {
            0..=16 => {
                let prev = lens[i];
                lens[i] = (prev + 17 - sym as u8) % 17;
                i += 1;
            }
            17 => {
                let run = 4 + bits.read_bits(4)? as usize;
                let run_end = i.checked_add(run).ok_or(Error::Corrupt)?;
                if run_end > end {
                    return Err(Error::Corrupt);
                }
                lens[i..run_end].fill(0);
                i = run_end;
            }
            18 => {
                let run = 20 + bits.read_bits(5)? as usize;
                let run_end = i.checked_add(run).ok_or(Error::Corrupt)?;
                if run_end > end {
                    return Err(Error::Corrupt);
                }
                lens[i..run_end].fill(0);
                i = run_end;
            }
            19 => {
                let run = 4 + bits.read_bits(1)? as usize;
                let sym2 = pretree.decode(bits)?;
                if sym2 > 16 {
                    return Err(Error::Corrupt);
                }
                let run_end = i.checked_add(run).ok_or(Error::Corrupt)?;
                if run_end > end {
                    return Err(Error::Corrupt);
                }
                let val = (lens[i] + 17 - sym2 as u8) % 17;
                lens[i..run_end].fill(val);
                i = run_end;
            }
            _ => return Err(Error::Corrupt),
        }
    }
    Ok(())
}

/// Stateful `[MS-CAB]` LZX folder decompressor.
///
/// One `LzxDecoder` is created per `CFFOLDER` and fed each `CFDATA` block in
/// order via [`LzxDecoder::decompress_chunk`]. Sliding window history, repeated
/// offset registers (`R0`/`R1`/`R2`), previous Huffman code lengths, and any
/// active block spanning chunk boundaries are preserved across `CFDATA` blocks
/// within the folder and reset only when a new folder begins.
#[derive(Clone, Debug)]
pub struct LzxDecoder {
    window: Vec<u8>,
    window_mask: usize,
    window_pos: usize,
    total_out: usize,

    r0: u32,
    r1: u32,
    r2: u32,

    main_elements: usize,
    main_lens: Vec<u8>,
    length_lens: [u8; NUM_LENGTH_ELEMENTS],
    main_tree: HuffmanTable,
    length_tree: HuffmanTable,
    aligned_tree: HuffmanTable,

    block_type: BlockType,
    block_remaining: usize,

    header_read: bool,
    e8_enabled: bool,
    e8_filesize: u32,
    e8_frame_offset: u64,
    frame_count: u32,
}

impl LzxDecoder {
    /// Create a new LZX decoder for a cabinet folder with window size
    /// `1 << window_bits` (`15..=21`, i.e., 32 KiB .. 2 MiB).
    pub fn new(window_bits: u16) -> Result<Self, Error> {
        let slots = position_slots(window_bits)?;
        let window_size = 1usize << window_bits;
        let main_elements = NUM_CHARS + 8 * slots;
        Ok(Self {
            window: vec![0u8; window_size],
            window_mask: window_size - 1,
            window_pos: 0,
            total_out: 0,
            r0: 1,
            r1: 1,
            r2: 1,
            main_elements,
            main_lens: vec![0u8; main_elements],
            length_lens: [0u8; NUM_LENGTH_ELEMENTS],
            main_tree: HuffmanTable::empty(),
            length_tree: HuffmanTable::empty(),
            aligned_tree: HuffmanTable::empty(),
            block_type: BlockType::None,
            block_remaining: 0,
            header_read: false,
            e8_enabled: false,
            e8_filesize: 0,
            e8_frame_offset: 0,
            frame_count: 0,
        })
    }

    /// Decompress a single `CFDATA` chunk (`payload`) producing `uncomp_len`
    /// uncompressed bytes (at most 32 KiB) and append them to `out`.
    pub fn decompress_chunk(&mut self, payload: &[u8], uncomp_len: usize, out: &mut Vec<u8>) -> Result<(), Error> {
        if uncomp_len > MAX_FRAME_SIZE {
            return Err(Error::Corrupt);
        }
        if uncomp_len == 0 {
            return Ok(());
        }

        let mut bits = BitReader::new(payload);
        if !self.header_read {
            let e8_flag = bits.read_bits(1)?;
            if e8_flag != 0 {
                let hi = bits.read_bits(16)?;
                let lo = bits.read_bits(16)?;
                self.e8_filesize = (hi << 16) | lo;
                self.e8_enabled = true;
            }
            self.header_read = true;
        }

        let out_start = out.len();
        out.reserve(uncomp_len);
        let mut chunk_remaining = uncomp_len;

        while chunk_remaining > 0 {
            if self.block_remaining == 0 {
                self.read_block_header(&mut bits)?;
            }

            match self.block_type {
                BlockType::Verbatim | BlockType::Aligned => {
                    self.decode_compressed_block(&mut bits, &mut chunk_remaining, out)?;
                }
                BlockType::Uncompressed => {
                    let take = self.block_remaining.min(chunk_remaining);
                    let raw = bits.read_raw_bytes(take)?;
                    self.emit_slice(raw, out);
                    self.block_remaining -= take;
                    chunk_remaining -= take;
                    if self.block_remaining == 0 {
                        // Realign to a 16-bit boundary after an odd-length
                        // uncompressed block before reading the next block header.
                        if (bits.byte_pos() & 1) != 0 && (chunk_remaining > 0 || bits.remaining_bytes() >= 1) {
                            let _ = bits.read_raw_bytes(1)?;
                        }
                        self.block_type = BlockType::None;
                    }
                }
                BlockType::None => return Err(Error::Corrupt),
            }
        }

        if out.len() - out_start != uncomp_len {
            return Err(Error::Corrupt);
        }

        if self.e8_enabled && self.frame_count < MAX_E8_FRAMES && uncomp_len > 10 {
            reverse_e8_translation(&mut out[out_start..out_start + uncomp_len], self.e8_frame_offset, self.e8_filesize);
        }
        self.e8_frame_offset = self.e8_frame_offset.saturating_add(uncomp_len as u64);
        self.frame_count = self.frame_count.saturating_add(1);

        Ok(())
    }

    fn read_block_header(&mut self, bits: &mut BitReader<'_>) -> Result<(), Error> {
        let block_type_raw = bits.read_bits(3)?;
        let block_len = bits.read_bits(24)? as usize;
        if block_len == 0 {
            return Err(Error::Corrupt);
        }
        match block_type_raw {
            1 => {
                // Verbatim block
                read_tree_lengths(bits, &mut self.main_lens, 0, NUM_CHARS)?;
                read_tree_lengths(bits, &mut self.main_lens, NUM_CHARS, self.main_elements)?;
                self.main_tree = HuffmanTable::build(&self.main_lens)?;
                read_tree_lengths(bits, &mut self.length_lens, 0, NUM_LENGTH_ELEMENTS)?;
                self.length_tree = HuffmanTable::build(&self.length_lens)?;
                self.block_type = BlockType::Verbatim;
                self.block_remaining = block_len;
            }
            2 => {
                // Aligned-offset block
                let mut aligned_lens = [0u8; NUM_ALIGNED_ELEMENTS];
                for l in &mut aligned_lens {
                    *l = bits.read_bits(3)? as u8;
                }
                self.aligned_tree = HuffmanTable::build(&aligned_lens)?;
                read_tree_lengths(bits, &mut self.main_lens, 0, NUM_CHARS)?;
                read_tree_lengths(bits, &mut self.main_lens, NUM_CHARS, self.main_elements)?;
                self.main_tree = HuffmanTable::build(&self.main_lens)?;
                read_tree_lengths(bits, &mut self.length_lens, 0, NUM_LENGTH_ELEMENTS)?;
                self.length_tree = HuffmanTable::build(&self.length_lens)?;
                self.block_type = BlockType::Aligned;
                self.block_remaining = block_len;
            }
            3 => {
                // Uncompressed block
                bits.align_for_uncompressed()?;
                self.r0 = bits.read_u32_le()?;
                self.r1 = bits.read_u32_le()?;
                self.r2 = bits.read_u32_le()?;
                self.block_type = BlockType::Uncompressed;
                self.block_remaining = block_len;
            }
            _ => return Err(Error::Corrupt),
        }
        Ok(())
    }

    fn decode_compressed_block(
        &mut self,
        bits: &mut BitReader<'_>,
        chunk_remaining: &mut usize,
        out: &mut Vec<u8>,
    ) -> Result<(), Error> {
        let aligned_block = self.block_type == BlockType::Aligned;
        let window_size = self.window.len();
        let window_mask = self.window_mask;

        while *chunk_remaining > 0 && self.block_remaining > 0 {
            let sym = self.main_tree.decode(bits)?;
            if usize::from(sym) < NUM_CHARS {
                let b = sym as u8;
                self.window[self.window_pos] = b;
                self.window_pos = (self.window_pos + 1) & window_mask;
                self.total_out = self.total_out.saturating_add(1);
                out.push(b);
                *chunk_remaining -= 1;
                self.block_remaining -= 1;
            } else {
                let match_header = usize::from(sym) - NUM_CHARS;
                let position_slot = match_header >> 3;
                let length_header = match_header & 7;
                let match_length = if length_header == 7 {
                    let footer = usize::from(self.length_tree.decode(bits)?);
                    9 + footer
                } else {
                    2 + length_header
                };

                if match_length > self.block_remaining || match_length > *chunk_remaining {
                    return Err(Error::Corrupt);
                }

                let match_offset = match position_slot {
                    0 => self.r0,
                    1 => {
                        core::mem::swap(&mut self.r0, &mut self.r1);
                        self.r0
                    }
                    2 => {
                        core::mem::swap(&mut self.r0, &mut self.r2);
                        self.r0
                    }
                    slot => {
                        if slot >= MAX_POSITION_SLOTS {
                            return Err(Error::Corrupt);
                        }
                        let extra = u32::from(EXTRA_BITS[slot]);
                        let formatted_offset = if aligned_block && extra >= 3 {
                            let verbatim = if extra > 3 { bits.read_bits(extra - 3)? << 3 } else { 0 };
                            let aligned = u32::from(self.aligned_tree.decode(bits)?);
                            BASE_POSITION[slot] + verbatim + aligned
                        } else {
                            let verbatim = bits.read_bits(extra)?;
                            BASE_POSITION[slot] + verbatim
                        };
                        let off = formatted_offset.checked_sub(2).ok_or(Error::Corrupt)?;
                        self.r2 = self.r1;
                        self.r1 = self.r0;
                        self.r0 = off;
                        off
                    }
                };

                let offset = match_offset as usize;
                if offset == 0 || offset > window_size || offset > self.total_out {
                    return Err(Error::Corrupt);
                }

                let mut src = (self.window_pos + window_size - offset) & window_mask;
                for _ in 0..match_length {
                    let b = self.window[src];
                    self.window[self.window_pos] = b;
                    src = (src + 1) & window_mask;
                    self.window_pos = (self.window_pos + 1) & window_mask;
                    out.push(b);
                }
                self.total_out = self.total_out.saturating_add(match_length);
                *chunk_remaining -= match_length;
                self.block_remaining -= match_length;
            }
        }

        if self.block_remaining == 0 {
            self.block_type = BlockType::None;
        }
        Ok(())
    }

    fn emit_slice(&mut self, raw: &[u8], out: &mut Vec<u8>) {
        out.extend_from_slice(raw);
        let window_size = self.window.len();
        let mut src = raw;
        while !src.is_empty() {
            let space = window_size - self.window_pos;
            let n = src.len().min(space);
            self.window[self.window_pos..self.window_pos + n].copy_from_slice(&src[..n]);
            self.window_pos = (self.window_pos + n) & self.window_mask;
            src = &src[n..];
        }
        self.total_out = self.total_out.saturating_add(raw.len());
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    //! Minimal clean-room LZX bitstream writer for synthetic unit tests.

    use super::*;
    use alloc::vec::Vec;

    pub struct BitWriter {
        out: Vec<u8>,
        cur_word: u16,
        pub bits_used: u32,
    }

    impl BitWriter {
        pub fn new() -> Self {
            Self { out: Vec::new(), cur_word: 0, bits_used: 0 }
        }

        pub fn write_bits(&mut self, val: u32, count: u32) {
            for shift in (0..count).rev() {
                let bit = ((val >> shift) & 1) as u16;
                self.cur_word = (self.cur_word << 1) | bit;
                self.bits_used += 1;
                if self.bits_used == 16 {
                    self.out.extend_from_slice(&self.cur_word.to_le_bytes());
                    self.cur_word = 0;
                    self.bits_used = 0;
                }
            }
        }

        /// Align to 16-bit boundary with 1..=16 zero bits for uncompressed block.
        pub fn align_for_uncompressed(&mut self) {
            let pad = 16 - self.bits_used;
            self.write_bits(0, pad);
        }

        pub fn write_raw_bytes(&mut self, bytes: &[u8]) {
            assert_eq!(self.bits_used, 0);
            self.out.extend_from_slice(bytes);
        }

        pub fn write_u32_le(&mut self, v: u32) {
            self.write_raw_bytes(&v.to_le_bytes());
        }

        /// Flush partial 16-bit word with 0..=15 zero bits (end of chunk).
        pub fn finish(mut self) -> Vec<u8> {
            if self.bits_used > 0 {
                let pad = 16 - self.bits_used;
                self.write_bits(0, pad);
            }
            self.out
        }
    }

    /// Compute canonical codes `(code, len)` for each symbol from `lengths`.
    pub fn canonical_codes(lengths: &[u8]) -> Vec<(u32, u8)> {
        let mut bl_count = [0u32; MAX_CODE_LEN + 1];
        for &l in lengths {
            if l > 0 {
                bl_count[usize::from(l)] += 1;
            }
        }
        let mut next_code = [0u32; MAX_CODE_LEN + 1];
        let mut code = 0u32;
        for len in 1..=MAX_CODE_LEN {
            code = (code + bl_count[len - 1]) << 1;
            next_code[len] = code;
        }
        let mut out = vec![(0u32, 0u8); lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l > 0 {
                let c = next_code[usize::from(l)];
                next_code[usize::from(l)] += 1;
                out[sym] = (c, l);
            }
        }
        out
    }

    /// Build a complete canonical Huffman length array of size `n` where all
    /// symbols in `active` receive valid non-zero lengths (1 for 1 symbol,
    /// balanced power-of-two lengths for 2+ symbols).
    pub fn build_lengths_for_active(n: usize, active: &[usize]) -> Vec<u8> {
        let mut lens = vec![0u8; n];
        if active.is_empty() {
            return lens;
        }
        if active.len() == 1 {
            lens[active[0]] = 1;
            return lens;
        }
        let k = active.len();
        let bits = (usize::BITS - (k - 1).leading_zeros()) as u8; // ceil(log2(k))
        let extra = (1usize << bits) - k;
        for (idx, &sym) in active.iter().enumerate() {
            lens[sym] = if idx < extra { bits - 1 } else { bits };
        }
        lens
    }

    /// Encode tree length deltas `prev[start..end] -> target[start..end]` using
    /// a pretree (exercising codes 0..16, 17, 18, and 19).
    pub fn write_tree_lengths(bw: &mut BitWriter, prev: &mut [u8], target: &[u8], start: usize, end: usize) {
        enum Op {
            Delta(u8),
            ZeroShort(u32), // code 17 + 4 bits (4..=19)
            ZeroLong(u32),  // code 18 + 5 bits (20..=51)
            Same(u32, u8),  // code 19 + 1 bit (4..=5) + delta
        }
        let mut ops = Vec::new();
        let mut used = [false; NUM_PRETREE_ELEMENTS];
        let mut i = start;
        while i < end {
            if target[i] == 0 {
                let mut z = 0;
                while i + z < end && target[i + z] == 0 && z < 51 {
                    z += 1;
                }
                if z >= 20 {
                    ops.push(Op::ZeroLong((z - 20) as u32));
                    used[18] = true;
                    i += z;
                    continue;
                } else if z >= 4 {
                    ops.push(Op::ZeroShort((z - 4) as u32));
                    used[17] = true;
                    i += z;
                    continue;
                }
            }
            // Check for same non-zero run of 4 or 5 elements.
            if target[i] != 0 {
                let mut r = 1;
                while i + r < end && target[i + r] == target[i] && r < 5 {
                    r += 1;
                }
                if r >= 4 {
                    let delta = (prev[i] + 17 - target[i]) % 17;
                    ops.push(Op::Same((r - 4) as u32, delta));
                    used[19] = true;
                    used[usize::from(delta)] = true;
                    i += r;
                    continue;
                }
            }
            let delta = (prev[i] + 17 - target[i]) % 17;
            ops.push(Op::Delta(delta));
            used[usize::from(delta)] = true;
            i += 1;
        }
        let active: Vec<usize> = (0..NUM_PRETREE_ELEMENTS).filter(|&s| used[s]).collect();
        let pre_lens = build_lengths_for_active(NUM_PRETREE_ELEMENTS, &active);
        let pre_codes = canonical_codes(&pre_lens);
        for &l in &pre_lens {
            bw.write_bits(u32::from(l), 4);
        }
        for op in ops {
            match op {
                Op::Delta(d) => {
                    let (c, l) = pre_codes[usize::from(d)];
                    bw.write_bits(c, u32::from(l));
                }
                Op::ZeroShort(extra) => {
                    let (c, l) = pre_codes[17];
                    bw.write_bits(c, u32::from(l));
                    bw.write_bits(extra, 4);
                }
                Op::ZeroLong(extra) => {
                    let (c, l) = pre_codes[18];
                    bw.write_bits(c, u32::from(l));
                    bw.write_bits(extra, 5);
                }
                Op::Same(extra, d) => {
                    let (c19, l19) = pre_codes[19];
                    bw.write_bits(c19, u32::from(l19));
                    bw.write_bits(extra, 1);
                    let (cd, ld) = pre_codes[usize::from(d)];
                    bw.write_bits(cd, u32::from(ld));
                }
            }
        }
        prev[start..end].copy_from_slice(&target[start..end]);
    }

    /// Forward Intel `0xE8` call translation (`[MS-PATCH]` preprocessing).
    pub fn forward_e8_translation(chunk: &mut [u8], chunk_offset: u64, filesize: u32) {
        let file_size = i64::from(filesize as i32);
        if file_size <= 0 || chunk_offset >= MAX_E8_OFFSET || chunk.len() <= 10 {
            return;
        }
        let limit = chunk.len() - 10;
        let mut i = 0usize;
        while i < limit {
            if chunk[i] == 0xE8 {
                let cur_pos = (chunk_offset + i as u64) as i64;
                let disp = i64::from(i32::from_le_bytes([chunk[i + 1], chunk[i + 2], chunk[i + 3], chunk[i + 4]]));
                let target = cur_pos + disp;
                if target >= 0 && target < file_size + cur_pos {
                    let encoded = if target >= file_size { disp - file_size } else { target };
                    chunk[i + 1..i + 5].copy_from_slice(&(encoded as i32).to_le_bytes());
                }
                i += 5;
            } else {
                i += 1;
            }
        }
    }

    /// Encode a single uncompressed LZX chunk (with E8 disabled).
    pub fn encode_uncompressed_chunk(data: &[u8], first_chunk: bool, r0: u32, r1: u32, r2: u32) -> Vec<u8> {
        let mut bw = BitWriter::new();
        if first_chunk {
            bw.write_bits(0, 1); // E8 disabled
        }
        bw.write_bits(3, 3); // Uncompressed
        bw.write_bits(data.len() as u32, 24);
        bw.align_for_uncompressed();
        bw.write_u32_le(r0);
        bw.write_u32_le(r1);
        bw.write_u32_le(r2);
        bw.write_raw_bytes(data);
        if (data.len() & 1) != 0 {
            bw.write_raw_bytes(&[0]);
        }
        bw.finish()
    }

    /// Find `(position_slot, extra_bits_value)` for a formatted offset `>= 3`.
    pub fn formatted_offset_slot(formatted: u32, num_slots: usize) -> (usize, u32) {
        for slot in (3..num_slots).rev() {
            if formatted >= BASE_POSITION[slot] {
                return (slot, formatted - BASE_POSITION[slot]);
            }
        }
        panic!("formatted offset {formatted} out of range for {num_slots} slots");
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;
    use alloc::vec;

    #[test]
    fn position_slot_tables_cover_exact_window_sizes() {
        for w in MIN_WINDOW_BITS..=MAX_WINDOW_BITS {
            let slots = position_slots(w).unwrap();
            let last = slots - 1;
            let end = BASE_POSITION[last] + (1u32 << EXTRA_BITS[last]);
            assert_eq!(end, 1u32 << w, "window_bits={w}");
        }
        assert_eq!(position_slots(0).unwrap_err(), Error::Compression);
        assert_eq!(position_slots(14).unwrap_err(), Error::Compression);
        assert_eq!(position_slots(22).unwrap_err(), Error::Compression);
    }

    #[test]
    fn uncompressed_blocks_odd_even_and_16bit_boundary_padding() {
        // 1. Exact [MS-PATCH] Structure Example: "abc" (odd length 3, 20 bytes total).
        let chunk = encode_uncompressed_chunk(b"abc", true, 1, 1, 1);
        assert_eq!(chunk.len(), 20);
        let mut dec = LzxDecoder::new(15).unwrap();
        let mut out = Vec::new();
        dec.decompress_chunk(&chunk, 3, &mut out).unwrap();
        assert_eq!(out, b"abc");

        // 2. Two consecutive uncompressed blocks inside one chunk:
        //    Block 1: odd length 5 ("hello") + 1 pad byte + reloads R0=10, R1=20, R2=30
        //    Block 2: even length 4 ("rust") starting at an already 16-bit aligned
        //    offset after 0 bits -> Wait: 3 + 24 = 27 bits -> 5 bits pad.
        //    Let's also test an exact 16-bit aligned uncompressed header:
        //    Verbatim block consuming 5 bits before the uncompressed block header
        //    (1 + 4 = 5 bits -> 5 + 3 + 24 = 32 bits -> 16-bit zero padding!).
        let mut bw = BitWriter::new();
        bw.write_bits(0, 1); // E8 off
        // Block 1: uncompressed "hello" (5 bytes, odd)
        bw.write_bits(3, 3);
        bw.write_bits(5, 24);
        bw.align_for_uncompressed();
        bw.write_u32_le(11);
        bw.write_u32_le(22);
        bw.write_u32_le(33);
        bw.write_raw_bytes(b"hello");
        bw.write_raw_bytes(&[0]); // odd pad byte -> realigned to 16-bit boundary
        // Block 2: uncompressed "rust" (4 bytes, even)
        bw.write_bits(3, 3);
        bw.write_bits(4, 24);
        bw.align_for_uncompressed();
        bw.write_u32_le(4);
        bw.write_u32_le(2);
        bw.write_u32_le(1);
        bw.write_raw_bytes(b"rust");
        let chunk2 = bw.finish();

        let mut dec = LzxDecoder::new(15).unwrap();
        let mut out = Vec::new();
        dec.decompress_chunk(&chunk2, 9, &mut out).unwrap();
        assert_eq!(out, b"hellorust");
        assert_eq!((dec.r0, dec.r1, dec.r2), (4, 2, 1));

        // 3. Uncompressed block whose header ends on an EXACT 16-bit boundary
        //    (requiring full 16 bits of zero padding per [MS-PATCH]).
        //    Construct a 1-symbol verbatim block emitting 1 byte 'X' where
        //    total bits before uncompressed block header == 16*k + 5 so that
        //    + 3 (type) + 24 (len) == 16*(k+2) (exact 16-bit boundary!).
        let main_n = NUM_CHARS + 8 * 30;
        let mut prev_main = vec![0u8; main_n];
        let mut prev_len = [0u8; NUM_LENGTH_ELEMENTS];
        let target_main = build_lengths_for_active(main_n, &[usize::from(b'X')]);
        let target_len = vec![0u8; NUM_LENGTH_ELEMENTS];
        let main_codes = canonical_codes(&target_main);

        // Compute how many literals of 'X' (1 bit each) make bits_used == 5.
        let mut probe = BitWriter::new();
        probe.write_bits(0, 1); // E8 off
        probe.write_bits(1, 3); // Verbatim
        probe.write_bits(1, 24); // dummy len
        write_tree_lengths(&mut probe, &mut prev_main.clone(), &target_main, 0, NUM_CHARS);
        write_tree_lengths(&mut probe, &mut prev_main.clone(), &target_main, NUM_CHARS, main_n);
        write_tree_lengths(&mut probe, &mut prev_len.clone(), &target_len, 0, NUM_LENGTH_ELEMENTS);
        let needed_lits = ((5 + 16 - probe.bits_used) % 16) as usize;
        let lit_count = if needed_lits == 0 { 16 } else { needed_lits };

        let mut bw = BitWriter::new();
        bw.write_bits(0, 1);
        bw.write_bits(1, 3);
        bw.write_bits(lit_count as u32, 24);
        write_tree_lengths(&mut bw, &mut prev_main, &target_main, 0, NUM_CHARS);
        write_tree_lengths(&mut bw, &mut prev_main, &target_main, NUM_CHARS, main_n);
        write_tree_lengths(&mut bw, &mut prev_len, &target_len, 0, NUM_LENGTH_ELEMENTS);
        let (cx, lx) = main_codes[usize::from(b'X')];
        for _ in 0..lit_count {
            bw.write_bits(cx, u32::from(lx));
        }
        assert_eq!(bw.bits_used, 5);
        // Uncompressed block header: 3 + 24 = 27 bits -> 5 + 27 = 32 -> bits_used == 0!
        bw.write_bits(3, 3);
        bw.write_bits(4, 24);
        assert_eq!(bw.bits_used, 0);
        bw.align_for_uncompressed(); // Must write 16 zero bits (2 bytes)!
        bw.write_u32_le(1);
        bw.write_u32_le(1);
        bw.write_u32_le(1);
        bw.write_raw_bytes(b"ABCD");
        let payload = bw.finish();

        let mut dec = LzxDecoder::new(15).unwrap();
        let mut out = Vec::new();
        dec.decompress_chunk(&payload, lit_count + 4, &mut out).unwrap();
        let mut expected = vec![b'X'; lit_count];
        expected.extend_from_slice(b"ABCD");
        assert_eq!(out, expected);
    }

    #[test]
    fn verbatim_block_literals_matches_r0_r1_r2_and_long_match() {
        // Test:
        // - Uncompressed seed block setting R0=4, R1=8, R2=12 and writing 16 bytes "0123456789ABCDEF"
        // - Verbatim block emitting:
        //   1. Literal '!'
        //   2. Match using R0 (slot 0, len 3 -> copies "DEF" at offset 4? Wait: after '!', tail is "CDEF!", offset 4 is "DEF!")
        //   3. Match using R1 (slot 1, swaps R0<->R1, len 4)
        //   4. Match using R2 (slot 2, swaps R0<->R2, len 2)
        //   5. Explicit offset match (slot >= 3) with long length (length_header == 7 -> length_tree footer = 11 -> len 20, overlapping RLE!)
        let window_bits = 15u16;
        let slots = position_slots(window_bits).unwrap();
        let main_n = NUM_CHARS + 8 * slots;
        let seed = b"0123456789ABCDEF";

        // Planned sequence after seed (16 bytes):
        // - Lit '!' -> len 17: "0123456789ABCDEF!"
        // - R0=4, len 3 -> src = 17-4 = 13 -> "DEF" -> len 20
        // - R1=8 (swap -> R0=8, R1=4), len 4 -> src = 20-8 = 12 -> "CDEF" -> len 24
        // - R2=12 (swap -> R0=12, R2=8), len 2 -> src = 24-12 = 12 -> "CD" -> len 26
        // - Explicit offset 1 (formatted = 3 -> slot 3, 0 extra bits), len 20 (header=7, footer=11):
        //   overlapping copy of 'D' 20 times -> len 46!
        // - R0 (now 1!), len 2 -> "DD" -> len 48!
        let sym_lit = usize::from(b'!');
        let sym_r0_len3 = NUM_CHARS + 1;
        let sym_r1_len4 = NUM_CHARS + (1 << 3) + 2;
        let sym_r2_len2 = NUM_CHARS + (2 << 3);
        let sym_slot3_long = NUM_CHARS + (3 << 3) + 7;
        let sym_r0_len2 = NUM_CHARS;

        let mut active_main = vec![sym_lit, sym_r0_len3, sym_r1_len4, sym_r2_len2, sym_slot3_long, sym_r0_len2];
        active_main.sort();
        let target_main = build_lengths_for_active(main_n, &active_main);
        let main_codes = canonical_codes(&target_main);

        let target_len = build_lengths_for_active(NUM_LENGTH_ELEMENTS, &[11]); // 9 + 11 = 20
        let len_codes = canonical_codes(&target_len);

        let mut prev_main = vec![0u8; main_n];
        let mut prev_len = [0u8; NUM_LENGTH_ELEMENTS];

        let mut bw = BitWriter::new();
        bw.write_bits(0, 1); // E8 off
        // Block 1: Uncompressed seed with R0=4, R1=8, R2=12
        bw.write_bits(3, 3);
        bw.write_bits(seed.len() as u32, 24);
        bw.align_for_uncompressed();
        bw.write_u32_le(4);
        bw.write_u32_le(8);
        bw.write_u32_le(12);
        bw.write_raw_bytes(seed);

        // Block 2: Verbatim block of 32 bytes (1 + 3 + 4 + 2 + 20 + 2)
        bw.write_bits(1, 3);
        bw.write_bits(32, 24);
        write_tree_lengths(&mut bw, &mut prev_main, &target_main, 0, NUM_CHARS);
        write_tree_lengths(&mut bw, &mut prev_main, &target_main, NUM_CHARS, main_n);
        write_tree_lengths(&mut bw, &mut prev_len, &target_len, 0, NUM_LENGTH_ELEMENTS);

        let emit = |bw: &mut BitWriter, codes: &[(u32, u8)], sym: usize| {
            let (c, l) = codes[sym];
            bw.write_bits(c, u32::from(l));
        };
        emit(&mut bw, &main_codes, sym_lit);
        emit(&mut bw, &main_codes, sym_r0_len3);
        emit(&mut bw, &main_codes, sym_r1_len4);
        emit(&mut bw, &main_codes, sym_r2_len2);
        emit(&mut bw, &main_codes, sym_slot3_long);
        emit(&mut bw, &len_codes, 11);
        emit(&mut bw, &main_codes, sym_r0_len2);
        let chunk = bw.finish();

        let mut dec = LzxDecoder::new(window_bits).unwrap();
        let mut out = Vec::new();
        dec.decompress_chunk(&chunk, 48, &mut out).unwrap();

        let mut expected = seed.to_vec();
        expected.push(b'!');
        expected.extend_from_slice(b"DEF");
        expected.extend_from_slice(b"CDEF");
        expected.extend_from_slice(b"CD");
        expected.extend(core::iter::repeat_n(b'D', 22));
        assert_eq!(out, expected);
    }

    #[test]
    fn aligned_offset_block_exercises_extra_lt3_eq3_and_gt3() {
        // Seed 64 distinct bytes, then decode an aligned-offset block with:
        // - offset 6 (formatted 8 -> slot 6, extra_bits = 2 < 3 -> verbatim only)
        // - offset 19 (formatted 21 -> slot 8, extra_bits = 3 == 3 -> aligned only, aligned_bits = 5)
        // - offset 43 (formatted 45 -> slot 10, extra_bits = 4 > 3 -> 1 verbatim bit (1) + aligned_bits = 5)
        let window_bits = 15u16;
        let slots = position_slots(window_bits).unwrap();
        let main_n = NUM_CHARS + 8 * slots;
        let seed: Vec<u8> = (0u8..64).collect();

        let (slot_lt3, extra_lt3) = formatted_offset_slot(6 + 2, slots);
        assert_eq!(EXTRA_BITS[slot_lt3], 2);
        let (slot_eq3, extra_eq3) = formatted_offset_slot(19 + 2, slots);
        assert_eq!(EXTRA_BITS[slot_eq3], 3);
        let (slot_gt3, extra_gt3) = formatted_offset_slot(43 + 2, slots);
        assert_eq!(EXTRA_BITS[slot_gt3], 4);

        let sym1 = NUM_CHARS + (slot_lt3 << 3) + (4 - 2); // len 4
        let sym2 = NUM_CHARS + (slot_eq3 << 3) + (5 - 2); // len 5
        let sym3 = NUM_CHARS + (slot_gt3 << 3) + (6 - 2); // len 6
        let mut active_main = vec![sym1, sym2, sym3];
        active_main.sort();
        let target_main = build_lengths_for_active(main_n, &active_main);
        let main_codes = canonical_codes(&target_main);
        let target_len = vec![0u8; NUM_LENGTH_ELEMENTS]; // empty length tree allowed!

        // Aligned offset tree: uniform 3-bit codes for all 8 values 0..8
        let aligned_lens = [3u8; NUM_ALIGNED_ELEMENTS];
        let aligned_codes = canonical_codes(&aligned_lens);

        let mut prev_main = vec![0u8; main_n];
        let mut prev_len = [0u8; NUM_LENGTH_ELEMENTS];

        let mut bw = BitWriter::new();
        bw.write_bits(0, 1); // E8 off
        // Uncompressed seed
        bw.write_bits(3, 3);
        bw.write_bits(seed.len() as u32, 24);
        bw.align_for_uncompressed();
        bw.write_u32_le(1);
        bw.write_u32_le(1);
        bw.write_u32_le(1);
        bw.write_raw_bytes(&seed);

        // Aligned-offset block: 4 + 5 + 6 = 15 bytes
        bw.write_bits(2, 3);
        bw.write_bits(15, 24);
        for &l in &aligned_lens {
            bw.write_bits(u32::from(l), 3);
        }
        write_tree_lengths(&mut bw, &mut prev_main, &target_main, 0, NUM_CHARS);
        write_tree_lengths(&mut bw, &mut prev_main, &target_main, NUM_CHARS, main_n);
        write_tree_lengths(&mut bw, &mut prev_len, &target_len, 0, NUM_LENGTH_ELEMENTS);

        // Match 1: extra_bits = 2 (< 3)
        let (c, l) = main_codes[sym1];
        bw.write_bits(c, u32::from(l));
        bw.write_bits(extra_lt3, 2);

        // Match 2: extra_bits = 3 (== 3 -> 0 verbatim bits + aligned tree)
        let (c, l) = main_codes[sym2];
        bw.write_bits(c, u32::from(l));
        let (ac, al) = aligned_codes[(extra_eq3 & 7) as usize];
        bw.write_bits(ac, u32::from(al));

        // Match 3: extra_bits = 4 (> 3 -> 1 verbatim bit + aligned tree)
        let (c, l) = main_codes[sym3];
        bw.write_bits(c, u32::from(l));
        bw.write_bits(extra_gt3 >> 3, 1);
        let (ac, al) = aligned_codes[(extra_gt3 & 7) as usize];
        bw.write_bits(ac, u32::from(al));

        let payload = bw.finish();

        let mut dec = LzxDecoder::new(window_bits).unwrap();
        let mut out = Vec::new();
        dec.decompress_chunk(&payload, seed.len() + 15, &mut out).unwrap();

        let mut expected = seed.clone();
        // Match 1: offset 6 from pos 64 -> bytes 58..62
        expected.extend_from_slice(&[58, 59, 60, 61]);
        // Match 2: offset 19 from pos 68 -> bytes 49..54
        expected.extend_from_slice(&[49, 50, 51, 52, 53]);
        // Match 3: offset 43 from pos 73 -> bytes 30..36
        expected.extend_from_slice(&[30, 31, 32, 33, 34, 35]);
        assert_eq!(out, expected);
    }

    #[test]
    fn e8_call_translation_roundtrip_and_window_isolation() {
        let filesize = 12_000_000u32;
        // Build a 2-frame (32768 + 128 = 32896 byte) payload containing E8 CALL
        // instructions with positive, negative, wrap-around, out-of-range, and
        // tail-guard displacements.
        let mut frame0 = vec![0x90u8; MAX_FRAME_SIZE];
        let place_e8 = |buf: &mut [u8], pos: usize, disp: i32| {
            buf[pos] = 0xE8;
            buf[pos + 1..pos + 5].copy_from_slice(&disp.to_le_bytes());
        };
        place_e8(&mut frame0, 10, 0x0000_1234); // positive in-range
        place_e8(&mut frame0, 40, -16); // negative in-range (target = 24 >= 0)
        place_e8(&mut frame0, 80, (filesize as i32) - 5); // wrap-around (target >= filesize)
        place_e8(&mut frame0, 120, -200); // out-of-range negative (target < 0)
        place_e8(&mut frame0, 160, (filesize as i32) + 50); // out-of-range positive
        place_e8(&mut frame0, MAX_FRAME_SIZE - 10, 0x0000_0100); // exact tail guard -> untouched!

        let mut frame1 = vec![0xCCu8; 128];
        place_e8(&mut frame1, 20, -32_700); // crosses back into frame 0!
        place_e8(&mut frame1, 60, 500);

        // Preprocess both frames with forward E8 translation.
        let mut pre0 = frame0.clone();
        forward_e8_translation(&mut pre0, 0, filesize);
        assert_ne!(pre0, frame0);
        // Tail guard byte at MAX_FRAME_SIZE - 10 must be unchanged.
        assert_eq!(&pre0[MAX_FRAME_SIZE - 10..MAX_FRAME_SIZE - 5], &frame0[MAX_FRAME_SIZE - 10..MAX_FRAME_SIZE - 5]);

        let mut pre1 = frame1.clone();
        forward_e8_translation(&mut pre1, MAX_FRAME_SIZE as u64, filesize);
        assert_ne!(pre1, frame1);

        // Encode chunk 0 as an uncompressed block with E8 header enabled.
        let mut bw0 = BitWriter::new();
        bw0.write_bits(1, 1); // E8 enabled
        bw0.write_bits(filesize >> 16, 16);
        bw0.write_bits(filesize & 0xFFFF, 16);
        bw0.write_bits(3, 3); // Uncompressed
        bw0.write_bits(pre0.len() as u32, 24);
        bw0.align_for_uncompressed();
        bw0.write_u32_le(1);
        bw0.write_u32_le(1);
        bw0.write_u32_le(1);
        bw0.write_raw_bytes(&pre0);
        let chunk0 = bw0.finish();

        // Encode chunk 1 as an uncompressed block of pre1.
        let chunk1 = encode_uncompressed_chunk(&pre1, false, 1, 1, 1);

        let mut dec = LzxDecoder::new(16).unwrap();
        let mut out = Vec::new();
        dec.decompress_chunk(&chunk0, frame0.len(), &mut out).unwrap();
        dec.decompress_chunk(&chunk1, frame1.len(), &mut out).unwrap();

        let mut expected = frame0.clone();
        expected.extend_from_slice(&frame1);
        assert_eq!(out, expected);

        // Verify window retains pre-translated bytes (not post-processed bytes),
        // so cross-chunk matches referencing frame0's E8 bytes copy pre0!
        assert_eq!(&dec.window[10..15], &pre0[10..15]);
        assert_ne!(&dec.window[10..15], &frame0[10..15]);
    }

    #[test]
    fn multi_chunk_folder_spanning_block_and_cross_chunk_match() {
        // Test across a 64 KiB window (window_bits = 16):
        // - Chunk 0 (32768 bytes): one uncompressed block of length 40000 (spans into chunk 1!).
        // - Chunk 1 (32768 bytes): remaining 7232 bytes of the uncompressed block,
        //   followed by a verbatim block that copies bytes from chunk 0 (offset 35000 > 32768!).
        let window_bits = 16u16;
        let slots = position_slots(window_bits).unwrap();
        let main_n = NUM_CHARS + 8 * slots;

        let mut uncomp_data = vec![0u8; 40_000];
        for (i, b) in uncomp_data.iter_mut().enumerate() {
            *b = ((i * 31 + 7) & 0xFF) as u8;
        }

        // Chunk 0: E8 off, block header for uncompressed 40000 bytes, first 32768 raw bytes.
        let mut bw0 = BitWriter::new();
        bw0.write_bits(0, 1);
        bw0.write_bits(3, 3);
        bw0.write_bits(40_000, 24);
        bw0.align_for_uncompressed();
        bw0.write_u32_le(1);
        bw0.write_u32_le(1);
        bw0.write_u32_le(1);
        bw0.write_raw_bytes(&uncomp_data[..MAX_FRAME_SIZE]);
        let chunk0 = bw0.finish();

        // Chunk 1: remaining 7232 raw bytes of uncompressed block, then a verbatim
        // block emitting 25536 bytes as 128 matches of length 199 + 1 match of length 64
        // at offset 35000 (crossing back into chunk 0!).
        let rem_len = 40_000 - MAX_FRAME_SIZE; // 7232 (even)
        let verb_len = MAX_FRAME_SIZE - rem_len; // 25536
        let offset = 35_000u32;
        let (slot, extra_val) = formatted_offset_slot(offset + 2, slots);
        let extra_bits = u32::from(EXTRA_BITS[slot]);

        let sym_long = NUM_CHARS + (slot << 3) + 7;
        let sym_r0_long = NUM_CHARS + 7;
        let target_main = build_lengths_for_active(main_n, &[sym_r0_long, sym_long]);
        let main_codes = canonical_codes(&target_main);
        // Lengths needed: 199 (footer 190) and 64 (footer 55)
        let target_len = build_lengths_for_active(NUM_LENGTH_ELEMENTS, &[55, 190]);
        let len_codes = canonical_codes(&target_len);

        let mut prev_main = vec![0u8; main_n];
        let mut prev_len = [0u8; NUM_LENGTH_ELEMENTS];

        let mut bw1 = BitWriter::new();
        bw1.write_raw_bytes(&uncomp_data[MAX_FRAME_SIZE..]);
        bw1.write_bits(1, 3); // Verbatim
        bw1.write_bits(verb_len as u32, 24);
        write_tree_lengths(&mut bw1, &mut prev_main, &target_main, 0, NUM_CHARS);
        write_tree_lengths(&mut bw1, &mut prev_main, &target_main, NUM_CHARS, main_n);
        write_tree_lengths(&mut bw1, &mut prev_len, &target_len, 0, NUM_LENGTH_ELEMENTS);

        // First match establishes R0 = 35000 (len 199).
        let (c, l) = main_codes[sym_long];
        bw1.write_bits(c, u32::from(l));
        let (lc, ll) = len_codes[190];
        bw1.write_bits(lc, u32::from(ll));
        bw1.write_bits(extra_val, extra_bits);

        // Next 127 matches reuse R0 (len 199 each).
        let (cr0, lr0) = main_codes[sym_r0_long];
        for _ in 1..128 {
            bw1.write_bits(cr0, u32::from(lr0));
            bw1.write_bits(lc, u32::from(ll));
        }
        // Final match of len 64 reusing R0 (128 * 199 + 64 = 25536).
        bw1.write_bits(cr0, u32::from(lr0));
        let (lc64, ll64) = len_codes[55];
        bw1.write_bits(lc64, u32::from(ll64));
        let chunk1 = bw1.finish();

        let mut dec = LzxDecoder::new(window_bits).unwrap();
        let mut out = Vec::new();
        dec.decompress_chunk(&chunk0, MAX_FRAME_SIZE, &mut out).unwrap();
        dec.decompress_chunk(&chunk1, MAX_FRAME_SIZE, &mut out).unwrap();

        assert_eq!(out.len(), 65_536);
        assert_eq!(&out[..40_000], &uncomp_data[..]);
        for i in 40_000..65_536 {
            assert_eq!(out[i], out[i - offset as usize], "mismatch at {i}");
        }
    }

    #[test]
    fn corrupt_and_truncated_streams_return_errors_without_panicking() {
        // Invalid window bits
        assert_eq!(LzxDecoder::new(14).unwrap_err(), Error::Compression);
        assert_eq!(LzxDecoder::new(22).unwrap_err(), Error::Compression);

        // Oversized frame request (> 32 KiB)
        let mut dec = LzxDecoder::new(15).unwrap();
        assert_eq!(dec.decompress_chunk(&[], MAX_FRAME_SIZE + 1, &mut Vec::new()).unwrap_err(), Error::Corrupt);

        // Truncated empty payload for non-zero output
        let mut dec = LzxDecoder::new(15).unwrap();
        assert_eq!(dec.decompress_chunk(&[], 10, &mut Vec::new()).unwrap_err(), Error::Truncated);

        // Invalid block type 0 and 4..=7
        for bad_type in [0u32, 4, 5, 6, 7] {
            let mut bw = BitWriter::new();
            bw.write_bits(0, 1); // E8 off
            bw.write_bits(bad_type, 3);
            bw.write_bits(16, 24);
            let payload = bw.finish();
            let mut dec = LzxDecoder::new(15).unwrap();
            assert_eq!(dec.decompress_chunk(&payload, 16, &mut Vec::new()).unwrap_err(), Error::Corrupt);
        }

        // Zero block length
        let mut bw = BitWriter::new();
        bw.write_bits(0, 1);
        bw.write_bits(3, 3);
        bw.write_bits(0, 24);
        let payload = bw.finish();
        let mut dec = LzxDecoder::new(15).unwrap();
        assert_eq!(dec.decompress_chunk(&payload, 4, &mut Vec::new()).unwrap_err(), Error::Corrupt);

        // Truncated uncompressed block payload
        let full = encode_uncompressed_chunk(b"hello world!", true, 1, 1, 1);
        for cut in 0..full.len() - 1 {
            let mut dec = LzxDecoder::new(15).unwrap();
            let err = dec.decompress_chunk(&full[..cut], 12, &mut Vec::new()).unwrap_err();
            assert!(matches!(err, Error::Truncated | Error::Corrupt), "cut={cut} err={err:?}");
        }

        // Over-subscribed Huffman table
        assert_eq!(HuffmanTable::build(&[1, 1, 1]).unwrap_err(), Error::Corrupt);
        // Incomplete multi-symbol Huffman table
        assert_eq!(HuffmanTable::build(&[2, 2]).unwrap_err(), Error::Corrupt);
        // Bit length > 16
        assert_eq!(HuffmanTable::build(&[17]).unwrap_err(), Error::Corrupt);

        // Match offset pointing before start of stream (R0=1 at total_out=0)
        let main_n = NUM_CHARS + 8 * 30;
        let sym_r0_len2 = NUM_CHARS; // slot 0, len 2
        let target_main = build_lengths_for_active(main_n, &[sym_r0_len2]);
        let target_len = vec![0u8; NUM_LENGTH_ELEMENTS];
        let main_codes = canonical_codes(&target_main);
        let mut prev_main = vec![0u8; main_n];
        let mut prev_len = [0u8; NUM_LENGTH_ELEMENTS];
        let mut bw = BitWriter::new();
        bw.write_bits(0, 1);
        bw.write_bits(1, 3);
        bw.write_bits(2, 24);
        write_tree_lengths(&mut bw, &mut prev_main, &target_main, 0, NUM_CHARS);
        write_tree_lengths(&mut bw, &mut prev_main, &target_main, NUM_CHARS, main_n);
        write_tree_lengths(&mut bw, &mut prev_len, &target_len, 0, NUM_LENGTH_ELEMENTS);
        let (c, l) = main_codes[sym_r0_len2];
        bw.write_bits(c, u32::from(l));
        let bad_match = bw.finish();
        let mut dec = LzxDecoder::new(15).unwrap();
        assert_eq!(dec.decompress_chunk(&bad_match, 2, &mut Vec::new()).unwrap_err(), Error::Corrupt);

        // Match length exceeding remaining chunk/block bytes
        let sym_lit = usize::from(b'A');
        let sym_r0_len8 = NUM_CHARS + 6; // slot 0, len 8
        let target_main = build_lengths_for_active(main_n, &[sym_lit, sym_r0_len8]);
        let main_codes = canonical_codes(&target_main);
        let mut prev_main = vec![0u8; main_n];
        let mut prev_len = [0u8; NUM_LENGTH_ELEMENTS];
        let mut bw = BitWriter::new();
        bw.write_bits(0, 1);
        bw.write_bits(1, 3);
        bw.write_bits(4, 24); // block claims 4 bytes, but we emit 1 + 8 = 9
        write_tree_lengths(&mut bw, &mut prev_main, &target_main, 0, NUM_CHARS);
        write_tree_lengths(&mut bw, &mut prev_main, &target_main, NUM_CHARS, main_n);
        write_tree_lengths(&mut bw, &mut prev_len, &target_len, 0, NUM_LENGTH_ELEMENTS);
        let (ca, la) = main_codes[sym_lit];
        bw.write_bits(ca, u32::from(la));
        let (cm, lm) = main_codes[sym_r0_len8];
        bw.write_bits(cm, u32::from(lm));
        let overflow_match = bw.finish();
        let mut dec = LzxDecoder::new(15).unwrap();
        assert_eq!(dec.decompress_chunk(&overflow_match, 4, &mut Vec::new()).unwrap_err(), Error::Corrupt);
    }

    #[test]
    fn delta_tree_updates_slow_16bit_huffman_codes_and_all_window_sizes() {
        // 1. Test slow-path Huffman decoding (> 10-bit code lengths, up to 16 bits):
        //    Construct a skewed complete binary tree of depth 16:
        //    lengths: 1, 2, 3, ..., 15, 16, 16 (17 active symbols; Kraft sum = 1).
        let mut skewed_lens = vec![0u8; 32];
        for (i, slot) in skewed_lens.iter_mut().enumerate().take(15) {
            *slot = (i + 1) as u8;
        }
        skewed_lens[15] = 16;
        skewed_lens[16] = 16;
        let table = HuffmanTable::build(&skewed_lens).unwrap();
        let codes = canonical_codes(&skewed_lens);

        let mut bw = BitWriter::new();
        for &(c, l) in codes.iter().take(17) {
            bw.write_bits(c, u32::from(l));
        }
        let encoded = bw.finish();
        let mut br = BitReader::new(&encoded);
        for sym in 0..17u16 {
            assert_eq!(table.decode(&mut br).unwrap(), sym);
        }

        // 2. Single-symbol tree rejects invalid bit `1`.
        let single = HuffmanTable::build(&[0, 1, 0]).unwrap();
        let mut bw = BitWriter::new();
        bw.write_bits(1, 1);
        let one_bit = bw.finish();
        let mut br = BitReader::new(&one_bit);
        assert_eq!(single.decode(&mut br).unwrap_err(), Error::Corrupt);

        // 3. Consecutive verbatim blocks with delta-coded tree lengths across
        //    all supported window sizes (15..=21, 32 KiB .. 2 MiB).
        for w in MIN_WINDOW_BITS..=MAX_WINDOW_BITS {
            let slots = position_slots(w).unwrap();
            let main_n = NUM_CHARS + 8 * slots;
            let mut prev_main = vec![0u8; main_n];
            let mut prev_len = [0u8; NUM_LENGTH_ELEMENTS];

            // Block 1: 4 active literals 'A', 'B', 'C', 'D' -> lengths [2, 2, 2, 2]
            // (exercises pretree code 19 same-run on 4 consecutive equal lengths!).
            let active1 = [usize::from(b'A'), usize::from(b'B'), usize::from(b'C'), usize::from(b'D')];
            let main1 = build_lengths_for_active(main_n, &active1);
            let codes1 = canonical_codes(&main1);
            let len_empty = vec![0u8; NUM_LENGTH_ELEMENTS];

            // Block 2: delta update from main1 -> 'A', 'B', 'E', 'F', plus slot 3 len 3 match!
            let sym_match = NUM_CHARS + (3 << 3) + (3 - 2);
            let active2 = [usize::from(b'A'), usize::from(b'B'), usize::from(b'E'), usize::from(b'F'), sym_match];
            let main2 = build_lengths_for_active(main_n, &active2);
            let codes2 = canonical_codes(&main2);

            let mut bw = BitWriter::new();
            bw.write_bits(0, 1); // E8 off
            // Block 1 (len 4): "ABCD"
            bw.write_bits(1, 3);
            bw.write_bits(4, 24);
            write_tree_lengths(&mut bw, &mut prev_main, &main1, 0, NUM_CHARS);
            write_tree_lengths(&mut bw, &mut prev_main, &main1, NUM_CHARS, main_n);
            write_tree_lengths(&mut bw, &mut prev_len, &len_empty, 0, NUM_LENGTH_ELEMENTS);
            for b in b"ABCD" {
                let (c, l) = codes1[usize::from(*b)];
                bw.write_bits(c, u32::from(l));
            }
            // Block 2 (len 6): "EF" + match(offset=1, len=3 -> "FFF") + "A"
            bw.write_bits(1, 3);
            bw.write_bits(6, 24);
            write_tree_lengths(&mut bw, &mut prev_main, &main2, 0, NUM_CHARS);
            write_tree_lengths(&mut bw, &mut prev_main, &main2, NUM_CHARS, main_n);
            write_tree_lengths(&mut bw, &mut prev_len, &len_empty, 0, NUM_LENGTH_ELEMENTS);
            for &sym in &[usize::from(b'E'), usize::from(b'F'), sym_match, usize::from(b'A')] {
                let (c, l) = codes2[sym];
                bw.write_bits(c, u32::from(l));
            }
            let payload = bw.finish();

            let mut dec = LzxDecoder::new(w).unwrap();
            let mut out = Vec::new();
            dec.decompress_chunk(&payload, 10, &mut out).unwrap();
            assert_eq!(out, b"ABCDEFFFFA", "window_bits={w}");
        }
    }
}
