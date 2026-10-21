//! The heap (Learn: `malloc`, `calloc`, `realloc`, `free`, `_msize`,
//! `_recalloc`, `_aligned_malloc` and family, `_set_new_mode`,
//! `_callnewh`): blocks of the process heap (`HeapAlloc`, 16-byte aligned on
//! x64), which `_get_heap_handle` returns.
//!
//! - `malloc(0)` returns a valid block; failure is NULL with `ENOMEM`; with
//!   `_set_new_mode(1)` a failed `malloc` first calls the new handler
//!   (`_callnewh`) and retries while it returns non-zero.
//! - `realloc(NULL, n)` is `malloc`; `realloc(p, 0)` frees `p` and returns
//!   NULL; a failed `realloc` leaves the block alone.
//! - `_aligned_malloc(size, alignment)`: `alignment` a power of two (else
//!   `EINVAL`); Poly keeps the heap block's address in the 8 bytes before
//!   the aligned block (its own layout: the blocks only go back to the
//!   `_aligned_*` functions).

use crate::global;
use crate::sys;
use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use poly_crt::errno::{EINVAL, ENOMEM};

fn heap() -> *mut u8 {
    // SAFETY: no preconditions.
    unsafe { sys::GetProcessHeap() }
}

/// The DLL's own Rust allocations (the process heap, like everything else).
pub struct ProcessHeap;

// SAFETY: HeapAlloc returns 16-byte aligned blocks; stricter alignments are
// refused.
unsafe impl GlobalAlloc for ProcessHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.align() > 16 {
            return core::ptr::null_mut();
        }
        // SAFETY: the process heap.
        unsafe { sys::HeapAlloc(heap(), 0, layout.size().max(1)) }
    }
    unsafe fn dealloc(&self, p: *mut u8, _: Layout) {
        // SAFETY: a block from `alloc`.
        unsafe { sys::HeapFree(heap(), 0, p) };
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if layout.align() > 16 {
            return core::ptr::null_mut();
        }
        // SAFETY: a block from `alloc`.
        unsafe { sys::HeapReAlloc(heap(), 0, p, size.max(1)) }
    }
}

/// `_set_new_mode`'s mode.
static NEW_MODE: AtomicI32 = AtomicI32::new(0);
/// The new handler (`_set_new_handler`): `int (*)(size_t)`.
pub static NEW_HANDLER: AtomicUsize = AtomicUsize::new(0);

/// Allocate `n` bytes with `flags`, honouring the new mode.
fn allocate(n: usize, flags: u32) -> *mut u8 {
    loop {
        // SAFETY: the process heap.
        let p = unsafe { sys::HeapAlloc(heap(), flags, n) };
        if !p.is_null() {
            return p;
        }
        // SAFETY: `_callnewh` has no preconditions.
        if NEW_MODE.load(Ordering::Acquire) == 0 || unsafe { _callnewh(n) } == 0 {
            global::set_errno(ENOMEM);
            return core::ptr::null_mut();
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn malloc(n: usize) -> *mut u8 {
    if n > isize::MAX as usize {
        global::set_errno(ENOMEM);
        return core::ptr::null_mut();
    }
    allocate(n, 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn calloc(count: usize, size: usize) -> *mut u8 {
    match count.checked_mul(size).filter(|&n| n <= isize::MAX as usize) {
        Some(n) => allocate(n, sys::HEAP_ZERO_MEMORY),
        None => {
            global::set_errno(ENOMEM);
            core::ptr::null_mut()
        }
    }
}

/// # Safety
/// `p` is null or a block of this heap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free(p: *mut u8) {
    if !p.is_null() {
        // SAFETY: the caller's block.
        unsafe { sys::HeapFree(heap(), 0, p) };
    }
}

/// # Safety
/// `p` is null or a block of this heap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn realloc(p: *mut u8, n: usize) -> *mut u8 {
    if p.is_null() {
        return malloc(n);
    }
    if n == 0 {
        // SAFETY: the caller's block.
        unsafe { free(p) };
        return core::ptr::null_mut();
    }
    loop {
        // SAFETY: the caller's block.
        let q = unsafe { sys::HeapReAlloc(heap(), 0, p, n) };
        if !q.is_null() {
            return q;
        }
        // SAFETY: as in `allocate`.
        if NEW_MODE.load(Ordering::Acquire) == 0 || unsafe { _callnewh(n) } == 0 {
            global::set_errno(ENOMEM);
            return core::ptr::null_mut();
        }
    }
}

/// `_msize`: the block's size; NULL is an invalid parameter (−1, `EINVAL`).
///
/// # Safety
/// `p` is null or a block of this heap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _msize(p: *mut u8) -> usize {
    if p.is_null() {
        global::invalid(EINVAL);
        return usize::MAX;
    }
    // SAFETY: the caller's block.
    unsafe { sys::HeapSize(heap(), 0, p) }
}

/// `_recalloc`: `realloc` of `count × size` bytes, the new bytes zeroed.
///
/// # Safety
/// `p` is null or a block of this heap.
pub unsafe extern "C" fn _recalloc(p: *mut u8, count: usize, size: usize) -> *mut u8 {
    let Some(n) = count.checked_mul(size) else {
        global::set_errno(ENOMEM);
        return core::ptr::null_mut();
    };
    // SAFETY: the caller's block.
    let old = if p.is_null() { 0 } else { unsafe { sys::HeapSize(heap(), 0, p) } };
    // SAFETY: as above.
    let q = unsafe { realloc(p, n) };
    if !q.is_null() && n > old {
        // SAFETY: the new block holds n bytes.
        unsafe { core::ptr::write_bytes(q.add(old), 0, n - old) };
    }
    q
}

/// `_expand`: Poly cannot grow a block in place; a shrink keeps it.
///
/// # Safety
/// `p` is a block of this heap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _expand(p: *mut u8, n: usize) -> *mut u8 {
    if p.is_null() {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's block.
    if n <= unsafe { sys::HeapSize(heap(), 0, p) } {
        return p;
    }
    global::set_errno(ENOMEM);
    core::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn _get_heap_handle() -> usize {
    heap() as usize
}

#[unsafe(no_mangle)]
pub extern "C" fn _heapmin() -> i32 {
    0
}

/// `_callnewh`: call the new handler with the size; its result (0 when
/// there is none).
///
/// # Safety
/// The handler, if any, has the documented signature.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _callnewh(n: usize) -> i32 {
    let h = NEW_HANDLER.load(Ordering::Acquire);
    if h == 0 {
        return 0;
    }
    // SAFETY: `int (__cdecl *)(size_t)`, registered by the program.
    let f: unsafe extern "C" fn(usize) -> i32 = unsafe { core::mem::transmute(h) };
    // SAFETY: as above.
    unsafe { f(n) }
}

/// `_set_new_mode`: the previous mode; anything but 0 or 1 is `EINVAL`.
pub extern "C" fn _set_new_mode(mode: i32) -> i32 {
    if mode != 0 && mode != 1 {
        global::invalid(EINVAL);
        return -1;
    }
    NEW_MODE.swap(mode, Ordering::AcqRel)
}

pub extern "C" fn _query_new_mode() -> i32 {
    NEW_MODE.load(Ordering::Acquire)
}

/// `_set_new_handler`'s storage, shared with the DLLs' exports of it.
pub fn set_new_handler(h: usize) -> usize {
    NEW_HANDLER.swap(h, Ordering::AcqRel)
}

// ----- aligned blocks --------------------------------------------------------------

/// `_aligned_offset_malloc(size, alignment, offset)`: `ptr + offset` is
/// aligned.
#[unsafe(no_mangle)]
pub extern "C" fn _aligned_offset_malloc(size: usize, alignment: usize, offset: usize) -> *mut u8 {
    if !alignment.is_power_of_two() || (offset != 0 && offset >= size) {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    let align = alignment.max(8);
    let Some(total) = size.checked_add(align).and_then(|n| n.checked_add(8)) else {
        global::set_errno(ENOMEM);
        return core::ptr::null_mut();
    };
    let raw = malloc(total);
    if raw.is_null() {
        return raw;
    }
    let base = raw as usize + 8 + offset;
    let aligned = (base + align - 1) & !(align - 1);
    let user = aligned - offset;
    // SAFETY: user - 8 >= raw and user + size <= raw + total.
    unsafe { ((user - 8) as *mut usize).write_unaligned(raw as usize) };
    user as *mut u8
}

#[unsafe(no_mangle)]
pub extern "C" fn _aligned_malloc(size: usize, alignment: usize) -> *mut u8 {
    _aligned_offset_malloc(size, alignment, 0)
}

/// # Safety
/// `p` is null or from `_aligned_malloc` and family.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _aligned_free(p: *mut u8) {
    if !p.is_null() {
        // SAFETY: the heap block's address is just before the block.
        unsafe { free(((p as usize - 8) as *const usize).read_unaligned() as *mut u8) };
    }
}

/// # Safety
/// As `_aligned_free`.
pub unsafe extern "C" fn _aligned_msize(p: *mut u8, alignment: usize, offset: usize) -> usize {
    if p.is_null() || !alignment.is_power_of_two() {
        global::invalid(EINVAL);
        return usize::MAX;
    }
    // SAFETY: as `_aligned_free`.
    let raw = unsafe { ((p as usize - 8) as *const usize).read_unaligned() };
    // SAFETY: the heap block.
    let total = unsafe { sys::HeapSize(heap(), 0, raw as *const u8) };
    let _ = offset;
    total - (p as usize - raw)
}

/// # Safety
/// As `_aligned_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _aligned_offset_realloc(p: *mut u8, size: usize, alignment: usize, offset: usize) -> *mut u8 {
    if p.is_null() {
        return _aligned_offset_malloc(size, alignment, offset);
    }
    if size == 0 {
        // SAFETY: the caller's block.
        unsafe { _aligned_free(p) };
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's block.
    let old = unsafe { _aligned_msize(p, alignment, offset) };
    let q = _aligned_offset_malloc(size, alignment, offset);
    if !q.is_null() {
        // SAFETY: both blocks hold at least min(old, size) bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(p, q, old.min(size));
            _aligned_free(p);
        }
    }
    q
}

/// # Safety
/// As `_aligned_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _aligned_realloc(p: *mut u8, size: usize, alignment: usize) -> *mut u8 {
    // SAFETY: the caller's block.
    unsafe { _aligned_offset_realloc(p, size, alignment, 0) }
}

/// A heap copy of `n` units and a NUL (`_strdup`, `_wcsdup`, the
/// environment).
pub fn dup<T: Copy + Default>(s: &[T]) -> *mut T {
    let bytes = core::mem::size_of_val(s) + core::mem::size_of::<T>();
    let p = malloc(bytes) as *mut T;
    if !p.is_null() {
        // SAFETY: the block holds s.len() + 1 units.
        unsafe {
            core::ptr::copy_nonoverlapping(s.as_ptr(), p, s.len());
            p.add(s.len()).write(T::default());
        }
    }
    p
}
