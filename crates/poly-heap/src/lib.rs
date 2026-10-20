//! Kernel heap.
//!
//! Two allocators behind one [`Heap`]:
//!
//! - **Small objects** (up to [`MAX_SMALL`] bytes, alignment up to the
//!   object's size class) come from *slabs*: power-of-two sized, naturally
//!   aligned runs of pages carved into equal slots of one size class. Each
//!   slab starts with a small header that holds its free list and in-use
//!   count; dealloc finds the header by rounding the pointer down to the slab
//!   size, so no lookup table is needed.
//! - **Large objects** (anything bigger, or more strictly aligned) are runs of
//!   whole pages taken straight from the [`PageSource`] and given back on
//!   dealloc. The page count is recomputed from the [`Layout`], which is why
//!   dealloc must get the exact layout the allocation was made with (the
//!   [`GlobalAlloc`](core::alloc::GlobalAlloc) contract already demands that).
//!
//! # Size classes
//!
//! [`CLASS_SIZES`] lists them: powers of two from 16 to 2048 plus the
//! midpoints 48, 96, … 1536, which bounds internal waste at about a third
//! instead of a half. A slot of size `s` is aligned to the largest power of
//! two dividing `s` (48 → 16, 1536 → 512), because slabs are aligned to at
//! least their own size. A request goes to the smallest class that is at
//! least `size` bytes *and* at least `align`-aligned; for power-of-two sizes
//! that is simply `max(size, align)`.
//!
//! A slab is one page when that gives at least 16 slots, otherwise
//! `16 × size` rounded up to a power of two (so 512 → 8 KiB, 2048 →
//! 32 KiB). The header (32 bytes) takes the first slot or two.
//!
//! # What the page source must provide
//!
//! Slabs are requested with `align == slab size`, i.e. up to
//! [`MAX_SLAB_BYTES`] (32 KiB). A buddy allocator gives that for free; a
//! virtual-range allocator has to round its cursor up. Large allocations
//! pass `max(layout.align(), PAGE_SIZE)`. Whenever the source cannot or does
//! not honour an alignment the heap returns null; it never hands out
//! misaligned memory.
//!
//! The page source must not allocate from the heap it feeds: the heap is
//! borrowed (and, behind [`LockedHeap`], locked) while it is called.
//!
//! # Returning memory
//!
//! When the last object of a slab is freed the slab goes back to the page
//! source, except that each class keeps one empty slab cached so an
//! alloc/free pair at a slab boundary does not hit the page source every
//! time. [`Heap::trim`] releases the cached slabs too.
//!
//! Nothing here allocates, and all state lives inside the [`Heap`] value and
//! the pages it owns, so a heap can sit in a `static` (see [`LockedHeap`]).

#![no_std]

mod lock;

pub use lock::{HeapGuard, LockedHeap, RawLock, RawSpinLock};

use core::alloc::Layout;
use core::ptr::{self, NonNull};

pub const PAGE_SIZE: usize = 4096;

/// Object sizes of the small-object classes, smallest first.
pub const CLASS_SIZES: [usize; CLASS_COUNT] = [16, 32, 48, 64, 96, 128, 192, 256, 384, 512, 768, 1024, 1536, 2048];
pub const CLASS_COUNT: usize = 14;

/// Largest size (and alignment) served from slabs.
pub const MAX_SMALL: usize = CLASS_SIZES[CLASS_COUNT - 1];

/// Largest slab, which is also the largest alignment slabs ask the page
/// source for.
pub const MAX_SLAB_BYTES: usize = slab_bytes(MAX_SMALL);

/// Fewest slots (header included) a slab is sized for.
const MIN_SLOTS: usize = 16;

/// Every class size is a multiple of this, so a size rounded up to it
/// indexes [`CLASS_LOOKUP`].
const GRANULE: usize = 16;

/// Where the heap gets its memory: runs of contiguous, writable pages.
///
/// # Safety
///
/// `alloc_pages(count, align)` must return either `None` or a pointer to
/// `count * PAGE_SIZE` readable and writable bytes that nothing else uses
/// until the run is passed back to `free_pages`. The heap stores its own
/// bookkeeping in those bytes. It checks the alignment itself (and gives a
/// misaligned run straight back), but trusts everything else.
pub unsafe trait PageSource {
    /// Allocate `count` (at least 1) contiguous pages starting at a multiple
    /// of `align`, a power of two no smaller than [`PAGE_SIZE`].
    fn alloc_pages(&mut self, count: usize, align: usize) -> Option<NonNull<u8>>;

    /// Give back a run of pages.
    ///
    /// # Safety
    ///
    /// `ptr` was returned by `alloc_pages(count, _)` on this source and has
    /// not been freed since. The heap always frees whole runs.
    unsafe fn free_pages(&mut self, ptr: NonNull<u8>, count: usize);
}

/// Per-class numbers, as reported by [`Heap::stats`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClassStats {
    pub object_size: usize,
    /// Slabs held, including a cached empty one.
    pub slabs: usize,
    pub objects_in_use: usize,
    /// Free slots in the slabs held.
    pub objects_free: usize,
}

/// A snapshot of the heap's bookkeeping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    /// Sum of `layout.size()` over live allocations: what callers asked
    /// for, not what they got after rounding up to a class or page.
    pub bytes_in_use: usize,
    /// Pages currently borrowed from the page source (slabs and large runs).
    pub pages_held: usize,
    pub large_allocations: usize,
    pub large_pages: usize,
    /// Allocations that returned null since the heap was created.
    pub failed_allocations: usize,
    pub classes: [ClassStats; CLASS_COUNT],
}

/// A small-object allocator over size-class slabs, plus whole-page runs for
/// everything else. See the crate documentation for the layout.
pub struct Heap<S> {
    source: S,
    classes: [ClassState; CLASS_COUNT],
    bytes_in_use: usize,
    large_allocations: usize,
    large_pages: usize,
    failed_allocations: usize,
}

// SAFETY: the raw pointers in `classes` point into pages the heap owns
// exclusively; nothing about them is tied to the thread that created them.
unsafe impl<S: Send> Send for Heap<S> {}

impl<S> core::fmt::Debug for Heap<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Heap")
            .field("bytes_in_use", &self.bytes_in_use)
            .field("large_allocations", &self.large_allocations)
            .field("large_pages", &self.large_pages)
            .finish_non_exhaustive()
    }
}

impl<S> Heap<S> {
    pub const fn new(source: S) -> Heap<S> {
        Heap {
            source,
            classes: [ClassState::EMPTY; CLASS_COUNT],
            bytes_in_use: 0,
            large_allocations: 0,
            large_pages: 0,
            failed_allocations: 0,
        }
    }

    pub fn source(&self) -> &S {
        &self.source
    }

    pub fn stats(&self) -> Stats {
        let mut classes = [ClassStats::default(); CLASS_COUNT];
        let mut slab_pages = 0;
        for ((out, state), class) in classes.iter_mut().zip(&self.classes).zip(&CLASSES) {
            *out = ClassStats {
                object_size: class.size,
                slabs: state.slabs,
                objects_in_use: state.in_use,
                objects_free: state.slabs * class.objects - state.in_use,
            };
            slab_pages += state.slabs * class.slab_bytes / PAGE_SIZE;
        }
        Stats {
            bytes_in_use: self.bytes_in_use,
            pages_held: slab_pages + self.large_pages,
            large_allocations: self.large_allocations,
            large_pages: self.large_pages,
            failed_allocations: self.failed_allocations,
            classes,
        }
    }
}

impl<S: PageSource> Heap<S> {
    /// Allocate memory for `layout`. Returns null when the page source runs
    /// dry or cannot provide the alignment.
    ///
    /// A zero-size layout gets a dangling, suitably aligned pointer that
    /// uses no memory; passing it back to [`Heap::dealloc`] does nothing.
    pub fn alloc(&mut self, layout: Layout) -> *mut u8 {
        let p = match Kind::of(layout) {
            Kind::Zero => return ptr::without_provenance_mut(layout.align()),
            Kind::Small(class) => self.alloc_small(class),
            Kind::Large { pages, align } => self.alloc_large(pages, align),
        };
        if p.is_null() {
            self.failed_allocations += 1;
        } else {
            self.bytes_in_use += layout.size();
        }
        p
    }

    /// Free memory returned by [`Heap::alloc`].
    ///
    /// # Safety
    ///
    /// `ptr` came from `alloc(layout)` (or [`Heap::realloc`] to `layout`) on
    /// this heap with exactly this `layout`, and has not been freed since.
    /// The layout picks the size class or page count, so a different one
    /// corrupts the heap.
    pub unsafe fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        match Kind::of(layout) {
            Kind::Zero => return,
            // SAFETY: forwarded from our caller.
            Kind::Small(class) => unsafe { self.dealloc_small(ptr, class) },
            // SAFETY: forwarded from our caller; the pages were a run of
            // exactly `pages` from the source.
            Kind::Large { pages, .. } => unsafe { self.dealloc_large(ptr, pages) },
        }
        self.bytes_in_use -= layout.size();
    }

    /// Resize an allocation, keeping its alignment. Stays in place when the
    /// new size still fits the same class (or the same number of pages);
    /// otherwise allocates, copies and frees. On failure returns null and
    /// leaves the old allocation alone.
    ///
    /// # Safety
    ///
    /// Same as [`Heap::dealloc`] for `ptr` and `layout`.
    pub unsafe fn realloc(&mut self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let Ok(new_layout) = Layout::from_size_align(new_size, layout.align()) else {
            self.failed_allocations += 1;
            return ptr::null_mut();
        };
        if Kind::of(layout) == Kind::of(new_layout) {
            self.bytes_in_use = self.bytes_in_use - layout.size() + new_size;
            return ptr;
        }
        let new = self.alloc(new_layout);
        if !new.is_null() {
            // SAFETY: both blocks are valid for the smaller size and are
            // distinct live allocations, so they do not overlap.
            unsafe { ptr::copy_nonoverlapping(ptr, new, layout.size().min(new_size)) };
            // SAFETY: forwarded from our caller.
            unsafe { self.dealloc(ptr, layout) };
        }
        new
    }

    /// Give every cached empty slab back to the page source. Returns the
    /// number of pages released.
    pub fn trim(&mut self) -> usize {
        let mut released = 0;
        for (i, class) in CLASSES.iter().enumerate() {
            let slab = core::mem::replace(&mut self.classes[i].cached, ptr::null_mut());
            if !slab.is_null() {
                // SAFETY: a cached slab is an empty slab we own and nothing
                // links to it any more.
                unsafe { self.release_slab(i, slab) };
                released += class.slab_bytes / PAGE_SIZE;
            }
        }
        released
    }

    fn alloc_small(&mut self, class: usize) -> *mut u8 {
        let mut slab = self.classes[class].partial;
        if slab.is_null() {
            slab = core::mem::replace(&mut self.classes[class].cached, ptr::null_mut());
            if slab.is_null() {
                slab = self.new_slab(class);
                if slab.is_null() {
                    return ptr::null_mut();
                }
            }
            // SAFETY: `slab` is an empty slab we own and is on no list.
            unsafe { self.classes[class].push(slab) };
        }
        let state = &mut self.classes[class];
        // SAFETY: slabs on the partial list are live slabs of this class with
        // at least one free slot, so `free` points at a free slot inside it.
        unsafe {
            let obj = (*slab).free;
            (*slab).free = (*obj).next;
            (*slab).in_use += 1;
            if (*slab).free.is_null() {
                state.unlink(slab);
            }
            state.in_use += 1;
            obj.cast()
        }
    }

    /// # Safety
    ///
    /// `ptr` is a live object of `class` handed out by this heap.
    unsafe fn dealloc_small(&mut self, ptr: *mut u8, class: usize) {
        let info = &CLASSES[class];
        let slab = ptr.map_addr(|a| a & !(info.slab_bytes - 1)).cast::<Slab>();
        let offset = ptr.addr() - slab.addr();
        debug_assert!(
            offset >= info.first && (offset - info.first).is_multiple_of(info.size),
            "bad pointer for this layout"
        );

        let state = &mut self.classes[class];
        // SAFETY: `ptr` lies in a slab of `class` (it was allocated there and
        // the slab is held while it has live objects), and slabs are aligned
        // to their size, so `slab` is its header. The object is ours again,
        // so we may store the free-list link in it.
        let now_empty = unsafe {
            let was_full = (*slab).free.is_null();
            let obj = ptr.cast::<FreeObject>();
            obj.write(FreeObject { next: (*slab).free });
            (*slab).free = obj;
            (*slab).in_use -= 1;
            let now_empty = (*slab).in_use == 0;
            if now_empty && !was_full {
                state.unlink(slab);
            } else if !now_empty && was_full {
                state.push(slab);
            }
            now_empty
        };
        state.in_use -= 1;

        if now_empty {
            if state.cached.is_null() {
                state.cached = slab;
            } else {
                // SAFETY: the slab is empty and no longer on any list.
                unsafe { self.release_slab(class, slab) };
            }
        }
    }

    /// Get a fresh slab for `class` from the page source and thread all its
    /// slots onto its free list. Null if the source fails.
    fn new_slab(&mut self, class: usize) -> *mut Slab {
        let info = &CLASSES[class];
        let pages = info.slab_bytes / PAGE_SIZE;
        let Some(run) = self.alloc_run(pages, info.slab_bytes) else {
            return ptr::null_mut();
        };
        let base = run.as_ptr();
        // Link the slots in address order, building the list from the back.
        let mut free = ptr::null_mut::<FreeObject>();
        for i in (0..info.objects).rev() {
            // SAFETY: `first + i * size + size <= slab_bytes`, so the slot is
            // inside the run we just got, and slots are at least 16-aligned.
            unsafe {
                let obj = base.add(info.first + i * info.size).cast::<FreeObject>();
                obj.write(FreeObject { next: free });
                free = obj;
            }
        }
        let slab = base.cast::<Slab>();
        // SAFETY: the header fits before the first slot at the start of the run.
        unsafe { slab.write(Slab { next: ptr::null_mut(), prev: ptr::null_mut(), free, in_use: 0 }) };
        self.classes[class].slabs += 1;
        slab
    }

    /// # Safety
    ///
    /// `slab` is an empty slab of `class` that is on no list.
    unsafe fn release_slab(&mut self, class: usize, slab: *mut Slab) {
        let pages = CLASSES[class].slab_bytes / PAGE_SIZE;
        self.classes[class].slabs -= 1;
        // SAFETY: slabs are whole runs of `pages` from the source.
        unsafe { self.source.free_pages(NonNull::new_unchecked(slab.cast()), pages) };
    }

    fn alloc_large(&mut self, pages: usize, align: usize) -> *mut u8 {
        match self.alloc_run(pages, align) {
            Some(run) => {
                self.large_allocations += 1;
                self.large_pages += pages;
                run.as_ptr()
            }
            None => ptr::null_mut(),
        }
    }

    /// # Safety
    ///
    /// `ptr` is a live large allocation of `pages` pages.
    unsafe fn dealloc_large(&mut self, ptr: *mut u8, pages: usize) {
        self.large_allocations -= 1;
        self.large_pages -= pages;
        // SAFETY: large allocations are whole runs of `pages` from the
        // source, returned unchanged by `alloc_large`.
        unsafe { self.source.free_pages(NonNull::new_unchecked(ptr), pages) };
    }

    /// A run of `pages` pages aligned to `align`, or `None`. A run the
    /// source misaligned goes straight back.
    fn alloc_run(&mut self, pages: usize, align: usize) -> Option<NonNull<u8>> {
        let run = self.source.alloc_pages(pages, align)?;
        if run.addr().get() & (align - 1) != 0 {
            // SAFETY: we just got exactly this run from the source.
            unsafe { self.source.free_pages(run, pages) };
            return None;
        }
        Some(run)
    }
}

/// Where an allocation of a given layout lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Zero,
    Small(usize),
    Large { pages: usize, align: usize },
}

impl Kind {
    fn of(layout: Layout) -> Kind {
        let (size, align) = (layout.size(), layout.align());
        if size == 0 {
            Kind::Zero
        } else if let Some(class) = class_for(size, align) {
            Kind::Small(class)
        } else {
            Kind::Large { pages: size.div_ceil(PAGE_SIZE), align: align.max(PAGE_SIZE) }
        }
    }
}

/// The smallest class with slots of at least `size` bytes aligned to at
/// least `align`, if there is one.
fn class_for(size: usize, align: usize) -> Option<usize> {
    if size > MAX_SMALL || align > MAX_SMALL {
        return None;
    }
    let mut class = CLASS_LOOKUP[size.div_ceil(GRANULE)] as usize;
    // Terminates: the last class is MAX_SMALL-aligned.
    while CLASSES[class].align < align {
        class += 1;
    }
    Some(class)
}

/// Geometry of one size class.
#[derive(Clone, Copy, Debug)]
struct Class {
    size: usize,
    /// Alignment every slot has: the largest power of two dividing `size`.
    align: usize,
    /// Size and alignment of a slab: a power of two, at least a page.
    slab_bytes: usize,
    /// Offset of the first slot, after the header.
    first: usize,
    /// Slots per slab.
    objects: usize,
}

impl Class {
    const fn new(size: usize) -> Class {
        let slab_bytes = slab_bytes(size);
        let first = size_of::<Slab>().div_ceil(size) * size;
        Class { size, align: 1 << size.trailing_zeros(), slab_bytes, first, objects: (slab_bytes - first) / size }
    }
}

const fn slab_bytes(size: usize) -> usize {
    let want = size * MIN_SLOTS;
    if want <= PAGE_SIZE { PAGE_SIZE } else { want.next_power_of_two() }
}

const CLASSES: [Class; CLASS_COUNT] = {
    let mut classes = [Class::new(CLASS_SIZES[0]); CLASS_COUNT];
    let mut i = 1;
    while i < CLASS_COUNT {
        classes[i] = Class::new(CLASS_SIZES[i]);
        i += 1;
    }
    classes
};

/// `CLASS_LOOKUP[n]` is the smallest class of at least `n * GRANULE` bytes.
const CLASS_LOOKUP: [u8; MAX_SMALL / GRANULE + 1] = {
    let mut table = [0u8; MAX_SMALL / GRANULE + 1];
    let (mut n, mut class) = (0, 0);
    while n < table.len() {
        while CLASS_SIZES[class] < n * GRANULE {
            class += 1;
        }
        table[n] = class as u8;
        n += 1;
    }
    table
};

/// Header at the start of every slab.
#[repr(C)]
struct Slab {
    /// Links in the class's partial list; null while the slab is full,
    /// cached or at the end of the list.
    next: *mut Slab,
    prev: *mut Slab,
    /// First free slot, null when the slab is full.
    free: *mut FreeObject,
    in_use: usize,
}

/// What a free slot holds.
struct FreeObject {
    next: *mut FreeObject,
}

/// Bookkeeping for one class. Full slabs are on no list: dealloc finds
/// them from the pointer.
#[derive(Clone, Copy)]
struct ClassState {
    /// Slabs with at least one free slot, most recently freed into first.
    /// Empty slabs do not stay here: they are cached or released.
    partial: *mut Slab,
    /// One completely empty slab kept back from the page source.
    cached: *mut Slab,
    slabs: usize,
    in_use: usize,
}

impl ClassState {
    const EMPTY: ClassState = ClassState { partial: ptr::null_mut(), cached: ptr::null_mut(), slabs: 0, in_use: 0 };

    /// # Safety
    ///
    /// `slab` is a live slab of this class that is on no list.
    unsafe fn push(&mut self, slab: *mut Slab) {
        // SAFETY: `slab` and the current head are live slab headers.
        unsafe {
            (*slab).prev = ptr::null_mut();
            (*slab).next = self.partial;
            if !self.partial.is_null() {
                (*self.partial).prev = slab;
            }
        }
        self.partial = slab;
    }

    /// # Safety
    ///
    /// `slab` is on this class's partial list.
    unsafe fn unlink(&mut self, slab: *mut Slab) {
        // SAFETY: `slab` and its neighbours are live slab headers.
        unsafe {
            let (prev, next) = ((*slab).prev, (*slab).next);
            if prev.is_null() {
                self.partial = next;
            } else {
                (*prev).next = next;
            }
            if !next.is_null() {
                (*next).prev = prev;
            }
            (*slab).prev = ptr::null_mut();
            (*slab).next = ptr::null_mut();
        }
    }
}

#[cfg(test)]
mod tests;
