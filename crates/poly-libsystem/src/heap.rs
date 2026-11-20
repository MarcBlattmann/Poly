//! `malloc`'s allocator: simple and small (ADR 0258).
//!
//! The sizes are those `specs/macos/libsystem_malloc.dylib/malloc_size.yaml`
//! and `malloc_good_size.yaml` record for x86-64 ([`good_size`]): a request
//! gets a multiple of 16 up to 1024 bytes, of 512 up to 32 KiB, then of
//! 32 KiB; `malloc_size(malloc(n))` is `malloc_good_size(n)`, and 0 for a
//! freed block or an address the heap does not own.
//!
//! Every block starts with a 16-byte header in front of the memory it
//! hands out: the block's size (with a check value in the upper bits) and
//! the offset of that memory from the block's start (16, or more for an
//! alignment above 16). A freed block goes onto the free list of its size
//! class (one list per class up to 32 KiB, one list searched for the exact
//! size above) and is taken from there first; otherwise blocks are carved
//! from the current arena, and an arena that runs out is replaced by one
//! from [`Morecore`] (what is left of the old one is cut into free blocks).
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
/// The smallest block: a header and 16 bytes.
const MIN_BLOCK: usize = 32;
/// Size classes up to 32 KiB: 64 of 16 bytes, then 62 of 512.
const SMALL_CLASSES: usize = 126;
/// How much an arena asks for at least.
pub const ARENA: usize = 1 << 20;
/// How many arenas the heap remembers for its ownership test.
const ARENAS: usize = 256;
/// The check value in the upper 16 bits of a block's size word.
const MAGIC: u64 = 0x9e1b << 48;
const SIZE_MASK: u64 = (1 << 48) - 1;

/// `malloc_good_size(n)`: what `malloc(n)` really gives (x86-64's classes);
/// `n` itself if rounding up would overflow.
pub fn good_size(n: usize) -> usize {
    let step = match n {
        0..=1024 => 16,
        1025..=32768 => 512,
        _ => 32768,
    };
    n.max(16).checked_next_multiple_of(step).unwrap_or(n)
}

/// The free-list class of a block holding `usable` bytes up to 32 KiB.
fn small_class(usable: usize) -> Option<usize> {
    match usable {
        16..=1024 => Some(usable / 16 - 1),
        1025..=32768 => Some(64 + usable / 512 - 3),
        _ => None,
    }
}

/// Why a pointer was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadPointer {
    /// Not a pointer this heap handed out, or freed twice.
    NotAllocated,
}

/// The allocator's state.
#[derive(Debug)]
pub struct Heap {
    /// The first free block of each small class, 0 if none.
    free: [usize; SMALL_CLASSES],
    /// Free blocks above 32 KiB, any size.
    large: usize,
    /// The arena blocks are carved from.
    at: usize,
    end: usize,
    /// The arenas: (start, end).
    arenas: [(usize, usize); ARENAS],
    arena_count: usize,
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

/// A free block's words: its total size, then the next free block.
///
/// # Safety
/// `block` is a free block of at least [`MIN_BLOCK`] bytes.
unsafe fn next_of(block: usize) -> usize {
    // SAFETY: the caller's contract.
    unsafe { (block as *const usize).add(1).read() }
}

impl Heap {
    pub const fn new() -> Self {
        Heap { free: [0; SMALL_CLASSES], large: 0, at: 0, end: 0, arenas: [(0, 0); ARENAS], arena_count: 0 }
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
        let need = size.checked_add(align - 16)?;
        let usable = good_size(need);
        if usable < need || usable > (SIZE_MASK as usize) / 2 {
            return None;
        }
        let total = usable + HEADER;
        // SAFETY: the caller's contract.
        let block = unsafe { self.take(total, m)? };
        let user = (block + HEADER).next_multiple_of(align);
        let header = user - HEADER;
        // SAFETY: the header lies inside the block, which is ours.
        unsafe {
            (header as *mut u64).write(total as u64 | MAGIC);
            (header as *mut u64).add(1).write((user - block) as u64);
        }
        Some(user as *mut u8)
    }

    /// A free block of `total` bytes (a header and a good size).
    ///
    /// # Safety
    /// As for [`Heap::alloc`].
    unsafe fn take(&mut self, total: usize, m: &mut dyn Morecore) -> Option<usize> {
        match small_class(total - HEADER) {
            Some(c) if self.free[c] != 0 => {
                let head = self.free[c];
                // SAFETY: a free block keeps the next one in its second word.
                self.free[c] = unsafe { next_of(head) };
                return Some(head);
            }
            Some(_) => {}
            None => {
                // The first free large block of exactly this size.
                let mut link: *mut usize = &raw mut self.large;
                // SAFETY: the list holds free blocks, each with its size in
                // its first word and the next in its second.
                unsafe {
                    while *link != 0 {
                        let b = *link;
                        if (b as *const usize).read() == total {
                            *link = next_of(b);
                            return Some(b);
                        }
                        link = (b as *mut usize).add(1);
                    }
                }
            }
        }
        if self.end - self.at < total {
            let (addr, len) = m.grow(total.max(ARENA))?;
            if len < total || !addr.is_multiple_of(16) {
                return None;
            }
            // SAFETY: the old arena's rest is ours.
            unsafe { self.retire() };
            self.at = addr;
            self.end = addr + len;
            if self.arena_count < ARENAS {
                self.arenas[self.arena_count] = (addr, addr + len);
            }
            self.arena_count += 1;
        }
        let block = self.at;
        self.at += total;
        Some(block)
    }

    /// Cut what is left of the arena into free blocks of the small classes.
    ///
    /// # Safety
    /// The arena is this heap's.
    unsafe fn retire(&mut self) {
        while self.end - self.at >= MIN_BLOCK {
            let rest = self.end - self.at - HEADER;
            // The largest class that fits the rest.
            let usable = if rest >= 32768 {
                32768
            } else if rest > 1024 {
                rest / 512 * 512
            } else {
                rest / 16 * 16
            };
            let total = usable.max(16) + HEADER;
            // SAFETY: the caller's contract.
            unsafe { self.push(self.at, total) };
            self.at += total;
        }
        self.at = self.end;
    }

    /// # Safety
    /// `block` is a block of `total` bytes nobody uses.
    unsafe fn push(&mut self, block: usize, total: usize) {
        let head = match small_class(total - HEADER) {
            Some(c) => &mut self.free[c],
            None => &mut self.large,
        };
        // SAFETY: the caller's contract: the block is free and at least 32
        // bytes long.
        unsafe {
            (block as *mut usize).write(total);
            (block as *mut usize).add(1).write(*head);
        }
        *head = block;
    }

    /// Whether `p` lies in one of the heap's arenas (all of them, while
    /// fewer than [`ARENAS`] were made; beyond that the header decides).
    fn owns(&self, p: usize) -> bool {
        if self.arena_count > ARENAS {
            return true;
        }
        self.arenas[..self.arena_count].iter().any(|&(a, e)| p >= a + HEADER && p < e)
    }

    /// The block size and the offset of the memory at `p`.
    ///
    /// # Safety
    /// `p` came from this heap, or lies in memory the heap's arenas cover.
    unsafe fn header(&self, p: *mut u8) -> Result<(usize, usize), BadPointer> {
        let user = p as usize;
        if !user.is_multiple_of(16) || !self.owns(user) {
            return Err(BadPointer::NotAllocated);
        }
        // SAFETY: inside an arena, past its first 16 bytes.
        let (word, offset) = unsafe {
            let h = (user - HEADER) as *const u64;
            (h.read(), h.add(1).read() as usize)
        };
        let size = (word & SIZE_MASK) as usize;
        if word & !SIZE_MASK != MAGIC || size < MIN_BLOCK || offset < HEADER || offset >= size {
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
        // SAFETY: the header is cleared so a second free (and
        // `malloc_size`) sees no block; the block is the caller's to give
        // back.
        unsafe {
            ((p as usize - HEADER) as *mut u64).write(0);
            self.push(block, size);
        }
        Ok(())
    }

    /// `malloc_size(p)`: how many bytes `p` can hold (0 for null).
    ///
    /// # Safety
    /// None beyond `p` being any address: what the heap does not own is
    /// refused without being read.
    pub unsafe fn usable_size(&self, p: *mut u8) -> Result<usize, BadPointer> {
        if p.is_null() {
            return Ok(0);
        }
        // SAFETY: `header` reads only inside the heap's arenas.
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

    /// `specs/macos/libsystem_malloc.dylib/malloc_good_size.yaml` and
    /// `malloc_size.yaml`, x86-64.
    #[test]
    fn the_size_classes_of_the_sheets() {
        for (n, want) in [
            (0, 16),
            (1, 16),
            (16, 16),
            (17, 32),
            (33, 48),
            (100, 112),
            (128, 128),
            (129, 144),
            (513, 528),
            (1000, 1008),
            (1025, 1536),
            (15000, 15360),
            (100_000, 131_072),
            (300_000, 327_680),
            (1_000_000, 1_015_808),
            (1 << 20, 1 << 20),
        ] {
            assert_eq!(good_size(n), want, "good_size({n})");
        }
        assert!(good_size((1 << 20) + 1) > 1 << 20);
        assert_eq!(good_size(usize::MAX), usize::MAX);
        let mut h = Heap::new();
        let mut m = host();
        let mut last = 0;
        for n in 0..=4096 {
            // SAFETY: the host arenas are the heap's.
            let p = unsafe { h.alloc(n, &mut m) }.unwrap();
            // SAFETY: from this heap.
            let size = unsafe { h.usable_size(p) }.unwrap();
            assert_eq!(size, good_size(n));
            assert!(size >= last && size % 16 == 0);
            last = size;
        }
    }

    #[test]
    fn foreign_and_freed_pointers_have_no_size() {
        let mut h = Heap::new();
        let mut m = host();
        let local = [0u128; 4];
        static GLOBAL: [u128; 2] = [0; 2];
        // SAFETY: the heap reads nothing it does not own.
        unsafe {
            let p = h.alloc(1 << 20, &mut m).unwrap();
            assert_eq!(h.usable_size(p), Ok(1 << 20));
            h.free(p).unwrap();
            assert!(h.usable_size(p).is_err());
            assert!(h.usable_size(local.as_ptr() as *mut u8).is_err());
            assert!(h.usable_size(GLOBAL.as_ptr() as *mut u8).is_err());
            assert!(h.usable_size(c"literal".as_ptr() as *mut u8).is_err());
            // The freed large block is reused for the same size.
            assert_eq!(h.alloc(1 << 20, &mut m), Some(p));
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
            let reused = h.alloc(32768, &mut m).unwrap() as usize;
            assert!(reused >= a && reused < a + len, "the old arena's rest is used");
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
