//! The NT heap: `RtlCreateHeap`, `RtlDestroyHeap`, `RtlAllocateHeap`,
//! `RtlFreeHeap`, `RtlReAllocateHeap`, `RtlSizeHeap`, `RtlLockHeap`,
//! `RtlUnlockHeap`, `RtlValidateHeap`, with the documented `HEAP_*` flags
//! (`winnt.h`; Learn's WDK `RtlCreateHeap`/`RtlAllocateHeap`/`RtlFreeHeap`
//! pages and the Win32 `HeapCreate`/`HeapAlloc`/`HeapReAlloc`/`HeapFree`/
//! `HeapSize`/`HeapValidate` pages, which describe the same semantics).
//!
//! # What a program sees
//!
//! - A heap handle is the address of the heap's first segment.
//! - Blocks are 16-byte aligned (`MEMORY_ALLOCATION_ALIGNMENT` on x64,
//!   `winnt.h`). A zero-byte request gets a unique block.
//! - `RtlSizeHeap` returns the size the caller asked for (not the rounded
//!   block size), `(SIZE_T)-1` for a pointer that is not a busy block of
//!   the heap.
//! - `HEAP_ZERO_MEMORY` zeroes a new block, and on reallocation the bytes
//!   past the old size.
//! - `HEAP_REALLOC_IN_PLACE_ONLY` fails a reallocation that would move.
//! - `HEAP_GENERATE_EXCEPTIONS` (at creation or per call): a failure
//!   *raises* `STATUS_NO_MEMORY` instead of returning null. The crate
//!   reports [`HeapError::NoMemory`] with [`raises`] telling the caller
//!   whether to raise; `ntdll` raises it.
//! - `HEAP_NO_SERIALIZE` (at creation or per call): no lock is taken.
//! - `HEAP_GROWABLE`: the heap adds segments as it fills; without it the
//!   heap is its first segment only.
//! - `HEAP_CREATE_ENABLE_EXECUTE`: the heap's memory is executable.
//! - Freeing a pointer that is not a busy block (twice, or not from this
//!   heap) returns `FALSE` and changes nothing; freeing null returns `TRUE`
//!   (to be confirmed by the spec agent, ADR 0071).
//! - `HEAP_TAIL_CHECKING_ENABLED`, `HEAP_FREE_CHECKING_ENABLED`,
//!   `HEAP_CREATE_ALIGN_16`, `HEAP_CREATE_ENABLE_TRACING` are accepted and
//!   change nothing (blocks are always 16-byte aligned).
//!
//! # Inside (Poly's own design; nothing here is visible to programs)
//!
//! Segments are runs of committed virtual memory from the [`Platform`],
//! carved into blocks with a 16-byte header (size and previous size in
//! 16-byte units, the unrequested bytes, flags and a tag), ending in a
//! sentinel header. Free blocks sit on 128 size-binned lists (exact sizes
//! up to 1 KiB, then four bins per power of two) with a bitmap, and merge
//! with free neighbours when freed (unless `HEAP_DISABLE_COALESCE_ON_FREE`).
//! Requests of [`BIG_THRESHOLD`] bytes or more in a growable heap get
//! virtual memory of their own, given back when freed; so does a growable
//! heap's extra segment once it is entirely free. Initial commit sizes are
//! not modelled: segments are committed whole (Poly's memory is demand-zero,
//! ADR 0063).

use crate::critsec::{CriticalSection, Waiter};
use core::ptr::{self, null_mut};

#[cfg(test)]
mod tests;

/// `HEAP_*` flags (`winnt.h`).
pub mod flags {
    pub const NO_SERIALIZE: u32 = 0x0000_0001;
    pub const GROWABLE: u32 = 0x0000_0002;
    pub const GENERATE_EXCEPTIONS: u32 = 0x0000_0004;
    pub const ZERO_MEMORY: u32 = 0x0000_0008;
    pub const REALLOC_IN_PLACE_ONLY: u32 = 0x0000_0010;
    pub const TAIL_CHECKING_ENABLED: u32 = 0x0000_0020;
    pub const FREE_CHECKING_ENABLED: u32 = 0x0000_0040;
    pub const DISABLE_COALESCE_ON_FREE: u32 = 0x0000_0080;
    pub const CREATE_ALIGN_16: u32 = 0x0001_0000;
    pub const CREATE_ENABLE_TRACING: u32 = 0x0002_0000;
    pub const CREATE_ENABLE_EXECUTE: u32 = 0x0004_0000;
}

/// What the heap needs from the system: committed virtual memory, and
/// (through [`Waiter`]) the lock's event.
pub trait Platform: Waiter {
    /// `size` bytes (a multiple of 64 KiB) of reserved and committed,
    /// zeroed, read/write memory (executable too if `execute`), aligned to
    /// 64 KiB; null if there is none.
    fn allocate(&self, size: usize, execute: bool) -> *mut u8;
    /// Give back an allocation [`allocate`](Self::allocate) made.
    fn release(&self, base: *mut u8, size: usize);
}

/// Why a heap call failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeapError {
    /// Not enough memory (`STATUS_NO_MEMORY`).
    NoMemory,
    /// The pointer is not a busy block of this heap.
    InvalidPointer,
    /// The handle is not a heap (`STATUS_INVALID_PARAMETER`).
    InvalidHeap,
    /// `HEAP_REALLOC_IN_PLACE_ONLY` and the block would have to move.
    WouldMove,
}

/// Whether a failure with these flags (the heap's and the call's) must be
/// raised as an exception (`HEAP_GENERATE_EXCEPTIONS`).
pub fn raises(heap_flags: u32, call_flags: u32) -> bool {
    (heap_flags | call_flags) & flags::GENERATE_EXCEPTIONS != 0
}

const UNIT: usize = 16;
const HEADER: usize = 16;
const MIN_UNITS: u32 = 2;
const NBINS: usize = 128;
const GRANULE: usize = 0x1_0000;
/// Requests of this many bytes or more get their own virtual memory in a
/// growable heap.
pub const BIG_THRESHOLD: usize = 0x7_F000;
const DEFAULT_RESERVE: usize = 0x10_0000;
const MAX_SEGMENT: usize = 0x100_0000;
const SIGNATURE: u64 = u64::from_le_bytes(*b"PolyHeap");

const BUSY: u8 = 1;
const BIG: u8 = 2;
const SENTINEL: u8 = 4;
const TAG: u8 = 0xA5;

#[repr(C)]
struct Block {
    /// Units of 16 bytes, header included; 0 for the sentinel.
    size: u32,
    /// Units of the block before; 0 for a segment's first block.
    prev_size: u32,
    /// Bytes of the block the caller did not ask for (busy blocks).
    unused: u32,
    flags: u8,
    tag: u8,
    _reserved: u16,
}

/// A free block's list links, in its payload.
#[repr(C)]
struct Links {
    next: *mut Block,
    prev: *mut Block,
}

#[repr(C)]
struct Segment {
    next: *mut Segment,
    base: *mut u8,
    size: usize,
    first: *mut Block,
    /// The sentinel header.
    end: *mut Block,
    /// The heap header lives at `base` (the first segment) or the memory
    /// is the caller's (`HeapBase`): never released on its own.
    pinned: bool,
}

#[repr(C)]
struct BigBlock {
    next: *mut BigBlock,
    prev: *mut BigBlock,
    alloc_size: usize,
    requested: usize,
    block: Block,
}

#[repr(C)]
struct HeapHeader {
    signature: u64,
    flags: u32,
    _pad: u32,
    lock: CriticalSection,
    segments: *mut Segment,
    big: *mut BigBlock,
    next_segment: usize,
    /// The first segment is the caller's memory (not released).
    caller_memory: bool,
    bitmap: [u64; 2],
    bins: [*mut Block; NBINS],
}

const HEAP_SIZE: usize = core::mem::size_of::<HeapHeader>().div_ceil(UNIT) * UNIT;
const SEGMENT_SIZE: usize = core::mem::size_of::<Segment>().div_ceil(UNIT) * UNIT;
const BIG_HEADER: usize = core::mem::size_of::<BigBlock>();

/// A heap handle: the address `RtlCreateHeap` returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heap(pub *mut u8);

fn round_up(v: usize, to: usize) -> Option<usize> {
    v.checked_add(to - 1).map(|x| x & !(to - 1))
}

fn bin_of(units: u32) -> usize {
    if units < 66 {
        return units.saturating_sub(MIN_UNITS) as usize;
    }
    let l = 31 - units.leading_zeros();
    let sub = (units >> (l - 2)) & 3;
    (64 + (l as usize - 6) * 4 + sub as usize).min(NBINS - 1)
}

/// Units of a block that holds `size` requested bytes.
fn units_for(size: usize) -> Option<u32> {
    let bytes = size.checked_add(HEADER)?.max(MIN_UNITS as usize * UNIT);
    u32::try_from(bytes.div_ceil(UNIT)).ok()
}

/// Lay a segment's blocks out in `[start, start + bytes)`: one free block
/// and the sentinel. Returns the segment record (written at `seg`).
///
/// # Safety
/// The range and `seg` are writable memory of the heap.
unsafe fn format_segment(seg: *mut Segment, base: *mut u8, size: usize, start: *mut u8, pinned: bool) -> bool {
    let end_addr = base as usize + size - HEADER;
    let first_units = (end_addr - start as usize) / UNIT;
    if first_units < MIN_UNITS as usize || first_units > u32::MAX as usize {
        return false;
    }
    let first = start as *mut Block;
    let end = end_addr as *mut Block;
    // SAFETY: the caller's contract: all inside the segment's memory.
    unsafe {
        first.write(Block { size: first_units as u32, prev_size: 0, unused: 0, flags: 0, tag: TAG, _reserved: 0 });
        end.write(Block {
            size: 0,
            prev_size: first_units as u32,
            unused: 0,
            flags: SENTINEL | BUSY,
            tag: TAG,
            _reserved: 0,
        });
        seg.write(Segment { next: null_mut(), base, size, first, end, pinned });
    }
    true
}

impl Heap {
    fn header(self) -> *mut HeapHeader {
        self.0 as *mut HeapHeader
    }

    /// `RtlCreateHeap(Flags, HeapBase, ReserveSize, CommitSize, Lock,
    /// Parameters)`: a new heap of `reserve` bytes (0: 1 MiB; rounded up to
    /// 64 KiB), in `base` (the caller's memory, `reserve` bytes, then never
    /// growable) or in memory from the platform.
    ///
    /// # Safety
    /// `base`, if given, is writable memory of `reserve` bytes, 16-byte
    /// aligned, that the heap may use until it is destroyed.
    pub unsafe fn create(p: &dyn Platform, flags: u32, base: Option<*mut u8>, reserve: usize) -> Option<Heap> {
        let (mem, size, caller, flags) = match base {
            Some(b) if !b.is_null() => {
                if !(b as usize).is_multiple_of(UNIT) {
                    return None;
                }
                (b, reserve & !(UNIT - 1), true, flags & !flags::GROWABLE)
            }
            _ => {
                let size = round_up(if reserve == 0 { DEFAULT_RESERVE } else { reserve }, GRANULE)?;
                let mem = p.allocate(size, flags & flags::CREATE_ENABLE_EXECUTE != 0);
                if mem.is_null() {
                    return None;
                }
                (mem, size, false, flags)
            }
        };
        if size < HEAP_SIZE + SEGMENT_SIZE + 4 * UNIT {
            if !caller {
                p.release(mem, size);
            }
            return None;
        }
        let h = mem as *mut HeapHeader;
        let seg = (mem as usize + HEAP_SIZE) as *mut Segment;
        let start = (seg as usize + SEGMENT_SIZE) as *mut u8;
        // SAFETY: `mem` is `size` writable bytes (ours or the caller's).
        unsafe {
            h.write(HeapHeader {
                signature: SIGNATURE,
                flags,
                _pad: 0,
                lock: CriticalSection::new(),
                segments: null_mut(),
                big: null_mut(),
                next_segment: size.clamp(DEFAULT_RESERVE, MAX_SEGMENT),
                caller_memory: caller,
                bitmap: [0; 2],
                bins: [null_mut(); NBINS],
            });
            if !format_segment(seg, mem, size, start, true) {
                if !caller {
                    p.release(mem, size);
                }
                return None;
            }
            (*h).segments = seg;
            let heap = Heap(mem);
            heap.insert((*seg).first);
            Some(heap)
        }
    }

    /// Whether `self` looks like a heap this crate made.
    ///
    /// # Safety
    /// `self.0` is null or readable for 8 bytes.
    pub unsafe fn is_valid(self) -> bool {
        // SAFETY: the caller's contract.
        !self.0.is_null()
            && (self.0 as usize).is_multiple_of(UNIT)
            && unsafe { (*self.header()).signature == SIGNATURE }
    }

    /// The flags the heap was created with.
    ///
    /// # Safety
    /// `self` is a valid heap.
    pub unsafe fn flags(self) -> u32 {
        // SAFETY: the caller's contract.
        unsafe { (*self.header()).flags }
    }

    /// Run `f` holding the heap's lock unless `HEAP_NO_SERIALIZE` (the
    /// heap's or the call's).
    ///
    /// # Safety
    /// `self` is a valid heap.
    unsafe fn locked<R>(self, p: &dyn Platform, call_flags: u32, f: impl FnOnce() -> R) -> R {
        // SAFETY: the caller's contract.
        let h = unsafe { &*self.header() };
        let serialize = (h.flags | call_flags) & flags::NO_SERIALIZE == 0;
        if serialize {
            h.lock.enter(p);
        }
        let r = f();
        if serialize {
            h.lock.leave(p);
        }
        r
    }

    /// `RtlLockHeap`.
    ///
    /// # Safety
    /// `self` is a valid heap.
    pub unsafe fn lock(self, p: &dyn Platform) {
        // SAFETY: the caller's contract.
        unsafe { (*self.header()).lock.enter(p) };
    }

    /// `RtlUnlockHeap`.
    ///
    /// # Safety
    /// `self` is a valid heap the caller locked.
    pub unsafe fn unlock(self, p: &dyn Platform) {
        // SAFETY: the caller's contract.
        unsafe { (*self.header()).lock.leave(p) };
    }

    // ----- free lists (the lock is held) ---------------------------------

    /// # Safety
    /// `b` is a free block of this heap, on no list.
    unsafe fn insert(self, b: *mut Block) {
        // SAFETY: the caller's contract; the heap's header and `b`'s payload
        // are the heap's memory.
        unsafe {
            let h = &mut *self.header();
            let bin = bin_of((*b).size);
            let links = b.add(1) as *mut Links;
            let head = h.bins[bin];
            (*links).next = head;
            (*links).prev = null_mut();
            if !head.is_null() {
                (*(head.add(1) as *mut Links)).prev = b;
            }
            h.bins[bin] = b;
            h.bitmap[bin / 64] |= 1 << (bin % 64);
        }
    }

    /// # Safety
    /// `b` is a free block on its bin's list.
    unsafe fn unlink(self, b: *mut Block) {
        // SAFETY: the caller's contract.
        unsafe {
            let h = &mut *self.header();
            let bin = bin_of((*b).size);
            let l = &*(b.add(1) as *const Links);
            if l.prev.is_null() {
                h.bins[bin] = l.next;
            } else {
                (*(l.prev.add(1) as *mut Links)).next = l.next;
            }
            if !l.next.is_null() {
                (*(l.next.add(1) as *mut Links)).prev = l.prev;
            }
            if h.bins[bin].is_null() {
                h.bitmap[bin / 64] &= !(1 << (bin % 64));
            }
        }
    }

    /// A free block of at least `units`, taken off its list.
    ///
    /// # Safety
    /// The heap is valid and locked.
    unsafe fn take_free(self, units: u32) -> Option<*mut Block> {
        // SAFETY: the caller's contract.
        let h = unsafe { &mut *self.header() };
        let start = bin_of(units);
        let mut bin = start;
        while bin < NBINS {
            let word = h.bitmap[bin / 64] & (!0u64 << (bin % 64));
            if word == 0 {
                bin = (bin / 64 + 1) * 64;
                continue;
            }
            bin = (bin / 64) * 64 + word.trailing_zeros() as usize;
            let mut b = h.bins[bin];
            while !b.is_null() {
                // SAFETY: list members are free blocks of the heap.
                unsafe {
                    if (*b).size >= units {
                        self.unlink(b);
                        return Some(b);
                    }
                    b = (*(b.add(1) as *const Links)).next;
                }
            }
            bin += 1;
        }
        None
    }

    /// The block after `b`.
    ///
    /// # Safety
    /// `b` is a non-sentinel block.
    unsafe fn next(b: *mut Block) -> *mut Block {
        // SAFETY: the caller's contract: a block's size leads to the next
        // header (at most the sentinel).
        unsafe { (b as *mut u8).add((*b).size as usize * UNIT) as *mut Block }
    }

    /// Make `b` (taken off its list, `b.size >= units`) exactly `units`
    /// long, freeing the rest when it is big enough to be a block.
    ///
    /// # Safety
    /// The heap is locked; `b` is one of its blocks, marked busy, not on a
    /// list.
    unsafe fn split(self, b: *mut Block, units: u32, coalesce: bool) {
        // SAFETY: the caller's contract.
        unsafe {
            let rest = (*b).size - units;
            if rest < MIN_UNITS {
                return;
            }
            (*b).size = units;
            let r = Heap::next(b);
            r.write(Block { size: rest, prev_size: units, unused: 0, flags: 0, tag: TAG, _reserved: 0 });
            let after = Heap::next(r);
            (*after).prev_size = rest;
            self.release_block(r, coalesce);
        }
    }

    /// Put a block that is not busy back: merge it with free neighbours
    /// (if `coalesce`) and list it.
    ///
    /// # Safety
    /// The heap is locked; `b` is one of its blocks, not on a list, not busy.
    unsafe fn release_block(self, mut b: *mut Block, coalesce: bool) {
        // SAFETY: the caller's contract; neighbours are found by the sizes
        // in the headers, which stay inside the segment.
        unsafe {
            if coalesce {
                let n = Heap::next(b);
                if (*n).flags & BUSY == 0 {
                    self.unlink(n);
                    (*b).size += (*n).size;
                    (*Heap::next(b)).prev_size = (*b).size;
                }
                if (*b).prev_size != 0 {
                    let prev = (b as *mut u8).sub((*b).prev_size as usize * UNIT) as *mut Block;
                    if (*prev).flags & BUSY == 0 {
                        self.unlink(prev);
                        (*prev).size += (*b).size;
                        (*Heap::next(prev)).prev_size = (*prev).size;
                        b = prev;
                    }
                }
            }
            (*b).flags = 0;
            (*b).unused = 0;
            self.insert(b);
        }
    }

    /// Add a segment big enough for `units`.
    ///
    /// # Safety
    /// The heap is valid, growable and locked.
    unsafe fn grow(self, p: &dyn Platform, units: u32) -> bool {
        // SAFETY: the caller's contract.
        let h = unsafe { &mut *self.header() };
        let need = (units as usize * UNIT).saturating_add(SEGMENT_SIZE + HEADER);
        let Some(size) = round_up(need.max(h.next_segment), GRANULE) else { return false };
        let mem = p.allocate(size, h.flags & flags::CREATE_ENABLE_EXECUTE != 0);
        if mem.is_null() {
            return false;
        }
        h.next_segment = (h.next_segment * 2).min(MAX_SEGMENT);
        let seg = mem as *mut Segment;
        // SAFETY: `mem` is a fresh allocation of `size` bytes.
        unsafe {
            if !format_segment(seg, mem, size, mem.add(SEGMENT_SIZE), false) {
                p.release(mem, size);
                return false;
            }
            (*seg).next = h.segments;
            h.segments = seg;
            self.insert((*seg).first);
        }
        true
    }

    /// The segment holding user pointer `ptr`, if any.
    ///
    /// # Safety
    /// The heap is valid and locked.
    unsafe fn segment_of(self, ptr: usize) -> Option<*mut Segment> {
        // SAFETY: the caller's contract; segments form a list.
        unsafe {
            let mut s = (*self.header()).segments;
            while !s.is_null() {
                if ptr >= (*s).first as usize + HEADER && ptr < (*s).end as usize {
                    return Some(s);
                }
                s = (*s).next;
            }
        }
        None
    }

    /// The big allocation whose user pointer is `ptr`.
    ///
    /// # Safety
    /// The heap is valid and locked.
    unsafe fn big_of(self, ptr: usize) -> Option<*mut BigBlock> {
        // SAFETY: the caller's contract.
        unsafe {
            let mut b = (*self.header()).big;
            while !b.is_null() {
                if (b as usize) + BIG_HEADER == ptr {
                    return Some(b);
                }
                b = (*b).next;
            }
        }
        None
    }

    /// The busy block whose user pointer is `ptr`, checked as far as can
    /// be without walking the segment: inside a segment, aligned, a tagged
    /// busy header whose size stays inside and whose next block agrees.
    ///
    /// # Safety
    /// The heap is valid and locked.
    unsafe fn busy_block(self, ptr: usize) -> Option<(*mut Block, *mut Segment)> {
        if !ptr.is_multiple_of(UNIT) {
            return None;
        }
        // SAFETY: the caller's contract.
        let seg = unsafe { self.segment_of(ptr)? };
        let b = (ptr - HEADER) as *mut Block;
        // SAFETY: `b` is inside the segment (checked by `segment_of`).
        unsafe {
            let ok = (*b).tag == TAG
                && (*b).flags & (BUSY | SENTINEL | BIG) == BUSY
                && (*b).size >= MIN_UNITS
                && (b as usize).checked_add((*b).size as usize * UNIT).is_some_and(|e| e <= (*seg).end as usize)
                && (*Heap::next(b)).prev_size == (*b).size;
            ok.then_some((b, seg))
        }
    }

    /// `RtlAllocateHeap`.
    ///
    /// # Safety
    /// `self` is a valid heap.
    pub unsafe fn allocate(self, p: &dyn Platform, call_flags: u32, size: usize) -> Result<*mut u8, HeapError> {
        // SAFETY: the caller's contract.
        let heap_flags = unsafe { self.flags() };
        let zero = call_flags & flags::ZERO_MEMORY != 0;
        let units = units_for(size).ok_or(HeapError::NoMemory)?;
        // SAFETY: the caller's contract; everything below runs locked.
        unsafe {
            self.locked(p, call_flags, || {
                if heap_flags & flags::GROWABLE != 0 && size >= BIG_THRESHOLD {
                    return self.allocate_big(p, size, zero);
                }
                let b = match self.take_free(units) {
                    Some(b) => b,
                    None if heap_flags & flags::GROWABLE != 0 && self.grow(p, units) => {
                        self.take_free(units).ok_or(HeapError::NoMemory)?
                    }
                    None => return Err(HeapError::NoMemory),
                };
                let coalesce = heap_flags & flags::DISABLE_COALESCE_ON_FREE == 0;
                // Busy before the split, so the remainder does not merge
                // back into it.
                (*b).flags = BUSY;
                self.split(b, units, coalesce);
                (*b).unused = ((*b).size as usize * UNIT - HEADER - size) as u32;
                let user = (b as *mut u8).add(HEADER);
                if zero {
                    ptr::write_bytes(user, 0, (*b).size as usize * UNIT - HEADER);
                }
                Ok(user)
            })
        }
    }

    /// # Safety
    /// The heap is valid and locked.
    unsafe fn allocate_big(self, p: &dyn Platform, size: usize, zero: bool) -> Result<*mut u8, HeapError> {
        // SAFETY: the caller's contract.
        let h = unsafe { &mut *self.header() };
        let alloc_size = size.checked_add(BIG_HEADER).and_then(|s| round_up(s, GRANULE)).ok_or(HeapError::NoMemory)?;
        let mem = p.allocate(alloc_size, h.flags & flags::CREATE_ENABLE_EXECUTE != 0);
        if mem.is_null() {
            return Err(HeapError::NoMemory);
        }
        let bb = mem as *mut BigBlock;
        // SAFETY: a fresh allocation, zeroed by the platform.
        unsafe {
            bb.write(BigBlock {
                next: h.big,
                prev: null_mut(),
                alloc_size,
                requested: size,
                block: Block { size: 0, prev_size: 0, unused: 0, flags: BUSY | BIG, tag: TAG, _reserved: 0 },
            });
            if !h.big.is_null() {
                (*h.big).prev = bb;
            }
            h.big = bb;
            let user = mem.add(BIG_HEADER);
            if zero {
                // Fresh memory is zero already; say so for the reader.
                debug_assert!(*user == 0);
            }
            Ok(user)
        }
    }

    /// # Safety
    /// The heap is valid and locked; `bb` is on its big list.
    unsafe fn free_big(self, p: &dyn Platform, bb: *mut BigBlock) {
        // SAFETY: the caller's contract.
        unsafe {
            let h = &mut *self.header();
            if (*bb).prev.is_null() {
                h.big = (*bb).next;
            } else {
                (*(*bb).prev).next = (*bb).next;
            }
            if !(*bb).next.is_null() {
                (*(*bb).next).prev = (*bb).prev;
            }
            p.release(bb as *mut u8, (*bb).alloc_size);
        }
    }

    /// `RtlFreeHeap`: `true` if `ptr` was a busy block of the heap (or
    /// null) and is now free.
    ///
    /// # Safety
    /// `self` is a valid heap; `ptr` is any value.
    pub unsafe fn free(self, p: &dyn Platform, call_flags: u32, ptr: *mut u8) -> bool {
        if ptr.is_null() {
            return true;
        }
        // SAFETY: the caller's contract.
        let heap_flags = unsafe { self.flags() };
        // SAFETY: the caller's contract; everything below runs locked.
        unsafe {
            self.locked(p, call_flags, || {
                if let Some(bb) = self.big_of(ptr as usize) {
                    self.free_big(p, bb);
                    return true;
                }
                let Some((b, seg)) = self.busy_block(ptr as usize) else { return false };
                (*b).flags = 0;
                self.release_block(b, heap_flags & flags::DISABLE_COALESCE_ON_FREE == 0);
                self.trim_segment(p, seg);
                true
            })
        }
    }

    /// Give an extra segment back once it is one free block.
    ///
    /// # Safety
    /// The heap is valid and locked.
    unsafe fn trim_segment(self, p: &dyn Platform, seg: *mut Segment) {
        // SAFETY: the caller's contract.
        unsafe {
            if (*seg).pinned {
                return;
            }
            let first = (*seg).first;
            if (*first).flags & BUSY != 0 || Heap::next(first) != (*seg).end {
                return;
            }
            self.unlink(first);
            let h = &mut *self.header();
            let mut link: *mut *mut Segment = &mut h.segments;
            while !(*link).is_null() {
                if *link == seg {
                    *link = (*seg).next;
                    break;
                }
                link = &mut (**link).next;
            }
            p.release((*seg).base, (*seg).size);
        }
    }

    /// `RtlSizeHeap`: the requested size of a busy block.
    ///
    /// # Safety
    /// `self` is a valid heap; `ptr` is any value.
    pub unsafe fn size(self, p: &dyn Platform, call_flags: u32, ptr: *const u8) -> Result<usize, HeapError> {
        // SAFETY: the caller's contract.
        unsafe {
            self.locked(p, call_flags, || {
                if let Some(bb) = self.big_of(ptr as usize) {
                    return Ok((*bb).requested);
                }
                let (b, _) = self.busy_block(ptr as usize).ok_or(HeapError::InvalidPointer)?;
                Ok((*b).size as usize * UNIT - HEADER - (*b).unused as usize)
            })
        }
    }

    /// `RtlReAllocateHeap`: resize a busy block, in place when it can
    /// (shrinking, or growing into a free block after it), else by moving
    /// it (unless `HEAP_REALLOC_IN_PLACE_ONLY`).
    ///
    /// # Safety
    /// `self` is a valid heap; `ptr` is any value.
    pub unsafe fn reallocate(
        self,
        p: &dyn Platform,
        call_flags: u32,
        ptr: *mut u8,
        size: usize,
    ) -> Result<*mut u8, HeapError> {
        // SAFETY: the caller's contract.
        let heap_flags = unsafe { self.flags() };
        let zero = call_flags & flags::ZERO_MEMORY != 0;
        let in_place = call_flags & flags::REALLOC_IN_PLACE_ONLY != 0;
        let units = units_for(size).ok_or(HeapError::NoMemory)?;
        let coalesce = heap_flags & flags::DISABLE_COALESCE_ON_FREE == 0;
        // In place, under the lock: Ok(Some) done, Ok(None) must move.
        // SAFETY: the caller's contract; runs locked.
        let done = unsafe {
            self.locked(p, call_flags, || -> Result<(Option<*mut u8>, usize), HeapError> {
                if let Some(bb) = self.big_of(ptr as usize) {
                    let old = (*bb).requested;
                    if size <= (*bb).alloc_size - BIG_HEADER {
                        (*bb).requested = size;
                        if zero && size > old {
                            ptr::write_bytes(ptr.add(old), 0, size - old);
                        }
                        return Ok((Some(ptr), old));
                    }
                    return Ok((None, old));
                }
                let (b, _) = self.busy_block(ptr as usize).ok_or(HeapError::InvalidPointer)?;
                let old = (*b).size as usize * UNIT - HEADER - (*b).unused as usize;
                if units > (*b).size {
                    let n = Heap::next(b);
                    if (*n).flags & BUSY != 0 || u64::from((*b).size) + u64::from((*n).size) < u64::from(units) {
                        return Ok((None, old));
                    }
                    self.unlink(n);
                    (*b).size += (*n).size;
                    (*Heap::next(b)).prev_size = (*b).size;
                }
                self.split(b, units, coalesce);
                (*b).unused = ((*b).size as usize * UNIT - HEADER - size) as u32;
                if zero && size > old {
                    ptr::write_bytes(ptr.add(old), 0, size - old);
                }
                Ok((Some(ptr), old))
            })?
        };
        match done {
            (Some(kept), _) => Ok(kept),
            (None, _) if in_place => Err(HeapError::WouldMove),
            (None, old) => {
                // SAFETY: the caller's contract; the old block stays busy
                // until the copy is made.
                unsafe {
                    let new = self.allocate(p, call_flags & !flags::ZERO_MEMORY, size)?;
                    ptr::copy_nonoverlapping(ptr, new, old.min(size));
                    if zero && size > old {
                        ptr::write_bytes(new.add(old), 0, size - old);
                    }
                    self.free(p, call_flags, ptr);
                    Ok(new)
                }
            }
        }
    }

    /// `RtlValidateHeap`: with a pointer, whether it is a busy block of the
    /// heap; without, whether every segment's blocks are consistent (sizes
    /// add up to the sentinel, neighbours agree, tags intact, every free
    /// block on the right list and every listed block free).
    ///
    /// # Safety
    /// `self` is a valid heap; `ptr` is any value.
    pub unsafe fn validate(self, p: &dyn Platform, call_flags: u32, ptr: *const u8) -> bool {
        // SAFETY: the caller's contract; runs locked.
        unsafe {
            self.locked(p, call_flags, || {
                if !ptr.is_null() {
                    return self.big_of(ptr as usize).is_some() || self.busy_block(ptr as usize).is_some();
                }
                self.check_all()
            })
        }
    }

    /// # Safety
    /// The heap is valid and locked.
    unsafe fn check_all(self) -> bool {
        // SAFETY: the caller's contract; every step stays inside a segment
        // after checking the size it follows.
        unsafe {
            let h = &*self.header();
            let mut free_blocks = 0usize;
            let mut s = h.segments;
            while !s.is_null() {
                let mut b = (*s).first;
                let mut prev = 0u32;
                loop {
                    if (*b).tag != TAG || (*b).prev_size != prev {
                        return false;
                    }
                    if b == (*s).end {
                        break;
                    }
                    if (*b).size < MIN_UNITS {
                        return false;
                    }
                    let Some(next) = (b as usize).checked_add((*b).size as usize * UNIT) else { return false };
                    if next > (*s).end as usize {
                        return false;
                    }
                    if (*b).flags & BUSY == 0 {
                        free_blocks += 1;
                    }
                    prev = (*b).size;
                    b = next as *mut Block;
                }
                s = (*s).next;
            }
            let mut listed = 0usize;
            for (bin, &head) in h.bins.iter().enumerate() {
                let mut b = head;
                let mut back = null_mut();
                while !b.is_null() {
                    if (*b).flags & BUSY != 0 || bin_of((*b).size) != bin {
                        return false;
                    }
                    let l = &*(b.add(1) as *const Links);
                    if l.prev != back {
                        return false;
                    }
                    listed += 1;
                    if listed > free_blocks {
                        return false;
                    }
                    back = b;
                    b = l.next;
                }
                if head.is_null() == (h.bitmap[bin / 64] & (1 << (bin % 64)) != 0) {
                    return false;
                }
            }
            listed == free_blocks
        }
    }

    /// `RtlDestroyHeap`: give every segment and big block back (the
    /// caller's memory, if it gave some, stays).
    ///
    /// # Safety
    /// `self` is a valid heap no other thread uses; it is gone afterwards.
    pub unsafe fn destroy(self, p: &dyn Platform) {
        // SAFETY: the caller's contract.
        unsafe {
            let h = self.header();
            let mut bb = (*h).big;
            while !bb.is_null() {
                let next = (*bb).next;
                p.release(bb as *mut u8, (*bb).alloc_size);
                bb = next;
            }
            (*h).lock.delete(p);
            let caller = (*h).caller_memory;
            let mut s = (*h).segments;
            (*h).signature = 0;
            // Release the first segment (which holds the header) last.
            let mut first = None;
            while !s.is_null() {
                let next = (*s).next;
                if (*s).base == self.0 {
                    first = Some(((*s).base, (*s).size));
                } else {
                    p.release((*s).base, (*s).size);
                }
                s = next;
            }
            if let (Some((base, size)), false) = (first, caller) {
                p.release(base, size);
            }
        }
    }
}
