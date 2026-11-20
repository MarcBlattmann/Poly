//! `malloc`'s allocator: simple and small (ADR 0258).
//!
//! Every block is a power of two from 32 bytes up, and starts with a
//! 16-byte header in front of the 16-byte aligned memory it hands out:
//! the block's size (with a check value in the upper bits) and the offset
//! of the handed-out memory from the block's start (16, or more for an
//! alignment above 16). A freed block goes onto the free list of its size
//! and is taken from there first; otherwise blocks are carved from the
//! current arena, and an arena that runs out is replaced by one from
//! [`Morecore`] (what is left of the old one is cut into free blocks).
//! Memory is never given back.
//!
//! What `malloc(3)` documents and this follows: `malloc(0)` returns a
//! unique pointer, `free(NULL)` does nothing, `realloc(NULL, n)` is
//! `malloc(n)`, `calloc` fails on an overflowing product, and memory is
//! 16-byte aligned. The dylib wraps one [`Heap`] in a lock.

/// Where arenas come from (`mmap`, or a static area).
pub trait Morecore {
    /// At least `min` bytes, 16-byte aligned: (address, length).
    fn grow(&mut self, min: usize) -> Option<(usize, usize)>;
}

/// The header in front of the memory handed out.
const HEADER: usize = 16;
/// The smallest block.
const MIN_BLOCK: usize = 32;
/// The largest block: 2^47 bytes.
const CLASSES: usize = 48;
/// How much an arena asks for at least.
pub const ARENA: usize = 1 << 20;
/// The check value in the upper 16 bits of a block's size word.
const MAGIC: u64 = 0x9e1b << 48;
const SIZE_MASK: u64 = (1 << 48) - 1;

/// Why a pointer was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadPointer {
    /// Not a pointer this heap handed out, or freed twice.
    NotAllocated,
}

/// The allocator's state.
#[derive(Debug)]
pub struct Heap {
    /// The first free block of each size class (2^k bytes), 0 if none.
    free: [usize; CLASSES],
    /// The arena blocks are carved from.
    at: usize,
    end: usize,
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

fn class_of(block: usize) -> usize {
    block.trailing_zeros() as usize
}

impl Heap {
    pub const fn new() -> Self {
        Heap { free: [0; CLASSES], at: 0, end: 0 }
    }

    /// `malloc(size)`: 16-byte aligned memory, or `None` (`ENOMEM`).
    ///
    /// # Safety
    /// The memory [`Morecore`] returns is this heap's alone and writable.
    pub unsafe fn alloc(&mut self, size: usize, m: &mut dyn Morecore) -> Option<*mut u8> {
        // SAFETY: the caller's contract.
        unsafe { self.alloc_aligned(size, 16, m) }
    }

    /// `posix_memalign`: memory aligned to `align` (a power of two).
    ///
    /// # Safety
    /// As for [`Heap::alloc`].
    pub unsafe fn alloc_aligned(&mut self, size: usize, align: usize, m: &mut dyn Morecore) -> Option<*mut u8> {
        let align = align.max(16);
        let need = size.checked_add(HEADER)?.checked_add(align - 16)?.max(MIN_BLOCK);
        let block_size = need.checked_next_power_of_two()?;
        let class = class_of(block_size);
        if class >= CLASSES {
            return None;
        }
        // SAFETY: the caller's contract.
        let block = unsafe { self.take(class, m)? };
        let user = (block + HEADER).next_multiple_of(align);
        let header = user - HEADER;
        // SAFETY: the header lies inside the block, which is ours.
        unsafe {
            (header as *mut u64).write(block_size as u64 | MAGIC);
            (header as *mut u64).add(1).write((user - block) as u64);
        }
        Some(user as *mut u8)
    }

    /// A free block of size class `class`.
    ///
    /// # Safety
    /// As for [`Heap::alloc`].
    unsafe fn take(&mut self, class: usize, m: &mut dyn Morecore) -> Option<usize> {
        let head = self.free[class];
        if head != 0 {
            // SAFETY: a free block keeps the next free block in its second
            // word (written by `push`).
            self.free[class] = unsafe { (head as *const usize).add(1).read() };
            return Some(head);
        }
        let size = 1usize << class;
        if self.end - self.at < size {
            let (addr, len) = m.grow(size.max(ARENA))?;
            if len < size || !addr.is_multiple_of(16) {
                return None;
            }
            // SAFETY: the old arena's rest is ours.
            unsafe { self.retire() };
            self.at = addr;
            self.end = addr + len;
        }
        // Blocks are aligned to 16; a block keeps its own size class.
        let block = self.at;
        self.at += size;
        Some(block)
    }

    /// Cut what is left of the arena into free blocks.
    ///
    /// # Safety
    /// The arena is this heap's.
    unsafe fn retire(&mut self) {
        while self.end - self.at >= MIN_BLOCK {
            let rest = self.end - self.at;
            let size = 1usize << (usize::BITS - 1 - rest.leading_zeros());
            // SAFETY: the caller's contract.
            unsafe { self.push(self.at, class_of(size)) };
            self.at += size;
        }
        self.at = self.end;
    }

    /// # Safety
    /// `block` is a block of size class `class` nobody uses.
    unsafe fn push(&mut self, block: usize, class: usize) {
        // SAFETY: the caller's contract: the block is free and at least 32
        // bytes long.
        unsafe {
            (block as *mut u64).write(0);
            (block as *mut usize).add(1).write(self.free[class]);
        }
        self.free[class] = block;
    }

    /// The block size and the offset of the memory at `p`.
    ///
    /// # Safety
    /// `p` came from this heap (checked as far as the header allows).
    unsafe fn header(&self, p: *mut u8) -> Result<(usize, usize), BadPointer> {
        let user = p as usize;
        if !user.is_multiple_of(16) || user < HEADER {
            return Err(BadPointer::NotAllocated);
        }
        // SAFETY: the caller's contract: the header is in front of `p`.
        let (word, offset) = unsafe {
            let h = (user - HEADER) as *const u64;
            (h.read(), h.add(1).read() as usize)
        };
        let size = (word & SIZE_MASK) as usize;
        if word & !SIZE_MASK != MAGIC
            || !size.is_power_of_two()
            || size < MIN_BLOCK
            || offset < HEADER
            || offset >= size
        {
            return Err(BadPointer::NotAllocated);
        }
        Ok((size, offset))
    }

    /// `free(p)`; a null pointer does nothing.
    ///
    /// # Safety
    /// `p` is null or came from this heap and is not used again.
    pub unsafe fn free(&mut self, p: *mut u8) -> Result<(), BadPointer> {
        if p.is_null() {
            return Ok(());
        }
        // SAFETY: the caller's contract.
        let (size, offset) = unsafe { self.header(p)? };
        let block = p as usize - offset;
        // SAFETY: the header is cleared so a second free is caught; the
        // block is the caller's to give back.
        unsafe {
            ((p as usize - HEADER) as *mut u64).write(0);
            self.push(block, class_of(size));
        }
        Ok(())
    }

    /// `malloc_size(p)`: how many bytes `p` can hold (0 for null).
    ///
    /// # Safety
    /// As for [`Heap::free`], without giving `p` up.
    pub unsafe fn usable_size(&self, p: *mut u8) -> Result<usize, BadPointer> {
        if p.is_null() {
            return Ok(0);
        }
        // SAFETY: the caller's contract.
        let (size, offset) = unsafe { self.header(p)? };
        Ok(size - offset)
    }

    /// `realloc(p, size)`: the same pointer if it already holds `size`
    /// bytes, else a new block with the contents copied (`p` freed).
    /// `None` leaves `p` as it was (`ENOMEM`).
    ///
    /// # Safety
    /// As for [`Heap::free`].
    pub unsafe fn realloc(
        &mut self,
        p: *mut u8,
        size: usize,
        m: &mut dyn Morecore,
    ) -> Result<Option<*mut u8>, BadPointer> {
        if p.is_null() {
            // SAFETY: the caller's contract.
            return Ok(unsafe { self.alloc(size, m) });
        }
        // SAFETY: the caller's contract.
        let have = unsafe { self.usable_size(p)? };
        if size <= have {
            return Ok(Some(p));
        }
        // SAFETY: the caller's contract.
        let Some(new) = (unsafe { self.alloc(size, m) }) else { return Ok(None) };
        // SAFETY: both blocks are ours and do not overlap; then the old one
        // is given back.
        unsafe {
            core::ptr::copy_nonoverlapping(p, new, have);
            self.free(p)?;
        }
        Ok(Some(new))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// Arenas from leaked host memory, at most `left` bytes in all.
    struct Host {
        left: usize,
        chunks: Vec<(usize, usize)>,
    }

    impl Morecore for Host {
        fn grow(&mut self, min: usize) -> Option<(usize, usize)> {
            let len = min.max(ARENA);
            if len > self.left {
                return None;
            }
            self.left -= len;
            let mem: &'static mut [u128] = Vec::leak(alloc::vec![0u128; len / 16]);
            let a = mem.as_mut_ptr() as usize;
            self.chunks.push((a, len));
            Some((a, len))
        }
    }

    fn host() -> Host {
        Host { left: 64 << 20, chunks: Vec::new() }
    }

    #[test]
    fn blocks_are_aligned_distinct_and_writable() {
        let mut h = Heap::new();
        let mut m = host();
        let mut ptrs = Vec::new();
        for n in [0usize, 1, 15, 16, 17, 100, 1000, 4096, 70_000] {
            // SAFETY: the host arenas are the heap's.
            let p = unsafe { h.alloc(n, &mut m) }.unwrap();
            assert_eq!(p as usize % 16, 0);
            // SAFETY: the block holds n bytes.
            unsafe { core::ptr::write_bytes(p, 0xab, n) };
            // SAFETY: from this heap.
            assert!(unsafe { h.usable_size(p) }.unwrap() >= n);
            ptrs.push((p as usize, n));
        }
        ptrs.sort();
        for w in ptrs.windows(2) {
            assert!(w[0].0 + w[0].1 <= w[1].0, "blocks overlap: {w:?}");
        }
    }

    #[test]
    fn freed_blocks_are_reused() {
        let mut h = Heap::new();
        let mut m = host();
        // SAFETY: the host arenas are the heap's; each pointer is freed once.
        unsafe {
            let a = h.alloc(40, &mut m).unwrap();
            h.free(a).unwrap();
            let b = h.alloc(48, &mut m).unwrap();
            assert_eq!(a, b);
            assert_eq!(h.free(core::ptr::null_mut()), Ok(()));
        }
    }

    #[test]
    fn double_free_and_foreign_pointers_are_caught() {
        let mut h = Heap::new();
        let mut m = host();
        // SAFETY: as above; the refused pointers' headers are readable.
        unsafe {
            let a = h.alloc(40, &mut m).unwrap();
            h.free(a).unwrap();
            assert_eq!(h.free(a), Err(BadPointer::NotAllocated));
            let mut local = [0u128; 4];
            let p = (local.as_mut_ptr() as *mut u8).add(32);
            assert_eq!(h.free(p), Err(BadPointer::NotAllocated));
            assert_eq!(h.free(p.add(1)), Err(BadPointer::NotAllocated));
        }
    }

    #[test]
    fn aligned_allocations() {
        let mut h = Heap::new();
        let mut m = host();
        for align in [16usize, 32, 64, 4096, 65536] {
            // SAFETY: as above.
            unsafe {
                let p = h.alloc_aligned(100, align, &mut m).unwrap();
                assert_eq!(p as usize % align, 0);
                assert!(h.usable_size(p).unwrap() >= 100);
                core::ptr::write_bytes(p, 1, 100);
                h.free(p).unwrap();
            }
        }
    }

    #[test]
    fn realloc_keeps_the_contents() {
        let mut h = Heap::new();
        let mut m = host();
        // SAFETY: as above.
        unsafe {
            let p = h.realloc(core::ptr::null_mut(), 10, &mut m).unwrap().unwrap();
            for k in 0..10 {
                *p.add(k) = k as u8;
            }
            // Fits its block: the same pointer.
            assert_eq!(h.realloc(p, 12, &mut m).unwrap(), Some(p));
            let q = h.realloc(p, 5000, &mut m).unwrap().unwrap();
            assert_ne!(p, q);
            for k in 0..10 {
                assert_eq!(*q.add(k), k as u8);
            }
            assert_eq!(h.free(p), Err(BadPointer::NotAllocated));
            h.free(q).unwrap();
        }
    }

    #[test]
    fn out_of_memory_is_none() {
        let mut h = Heap::new();
        let mut m = Host { left: ARENA, chunks: Vec::new() };
        // SAFETY: as above.
        unsafe {
            assert!(h.alloc(ARENA, &mut m).is_none());
            assert!(h.alloc(usize::MAX - 8, &mut m).is_none());
            let p = h.alloc(100, &mut m).unwrap();
            assert!(!p.is_null());
        }
    }

    #[test]
    fn a_new_arena_keeps_the_old_ones_rest() {
        let mut h = Heap::new();
        let mut m = host();
        // SAFETY: as above.
        unsafe {
            // Leave 1 MiB - 64 KiB - 32 of the first arena, then ask for more
            // than that: the rest becomes free blocks.
            h.alloc(65_536 - 16, &mut m).unwrap();
            h.alloc(16, &mut m).unwrap();
            h.alloc(ARENA - 16, &mut m).unwrap();
            assert_eq!(m.chunks.len(), 2);
            let (a, len) = m.chunks[0];
            let small = h.alloc(1000, &mut m).unwrap() as usize;
            assert!(small >= a && small < a + len, "the old arena's rest is used");
        }
    }

    #[test]
    fn many_allocations_and_frees() {
        let mut h = Heap::new();
        let mut m = host();
        let mut live: Vec<(*mut u8, usize, u8)> = Vec::new();
        let mut seed = 12345u32;
        for round in 0..5000u32 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let n = (seed >> 8) as usize % 3000;
            // SAFETY: as above; every live block is checked before it is freed.
            unsafe {
                if seed & 1 == 0 || live.is_empty() {
                    let p = h.alloc(n, &mut m).unwrap();
                    core::ptr::write_bytes(p, round as u8, n);
                    live.push((p, n, round as u8));
                } else {
                    let (p, n, v) = live.swap_remove((seed as usize >> 4) % live.len());
                    for k in 0..n {
                        assert_eq!(*p.add(k), v);
                    }
                    h.free(p).unwrap();
                }
            }
        }
    }
}
