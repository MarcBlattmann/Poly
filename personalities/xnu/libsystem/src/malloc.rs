//! libsystem_malloc: `malloc(3)` and friends over the host-tested
//! allocator of `poly_libsystem::heap`. Arenas come from `mmap`; while the
//! kernel has no `mmap` for Mach-O programs (it comes with ADR 0246) they
//! come from a zero-filled area of this library's own data.

use core::ffi::c_void;
use poly_libsystem::heap::{ARENA, Heap, Morecore};
use poly_libsystem::sys::{errno::*, mman};

unsafe extern "C" {
    fn mmap(addr: *mut c_void, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut c_void;
}

/// The area used when `mmap` is not there: 8 MiB of zero fill.
const STATIC: usize = 8 << 20;
#[repr(C, align(4096))]
struct Area([u8; STATIC]);
static mut AREA: Area = Area([0; STATIC]);
static mut AREA_USED: usize = 0;

struct Pages;

impl Morecore for Pages {
    fn grow(&mut self, min: usize) -> Option<(usize, usize)> {
        let len = min.max(ARENA).next_multiple_of(4096);
        // SAFETY: anonymous private memory; no file.
        let p = unsafe {
            mmap(
                core::ptr::null_mut(),
                len,
                mman::PROT_READ | mman::PROT_WRITE,
                mman::MAP_PRIVATE | mman::MAP_ANON,
                -1,
                0,
            )
        };
        if p as usize != mman::FAILED && !p.is_null() {
            return Some((p as usize, len));
        }
        // SAFETY: single-threaded (v0): nobody else touches the area's
        // bookkeeping.
        unsafe {
            let used = AREA_USED;
            if STATIC - used < len {
                return None;
            }
            AREA_USED = used + len;
            Some(((&raw mut AREA) as usize + used, len))
        }
    }
}

static HEAP: crate::Global<Heap> = crate::Global::new(Heap::new());

fn heap() -> &'static mut Heap {
    // SAFETY: single-threaded (v0); every entry point takes the heap once.
    unsafe { HEAP.get() }
}

fn bad(what: &[u8]) -> ! {
    let _ = what;
    crate::die(b"poly libSystem: free or realloc of a pointer malloc did not return\n")
}

fn nomem<T>() -> *mut T {
    crate::set_errno(ENOMEM);
    core::ptr::null_mut()
}

#[unsafe(no_mangle)]
unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
    // SAFETY: the arenas are the heap's own.
    match unsafe { heap().alloc(size, &mut Pages) } {
        Some(p) => p.cast(),
        None => nomem(),
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn calloc(n: usize, size: usize) -> *mut c_void {
    let Some(total) = n.checked_mul(size) else { return nomem() };
    // SAFETY: as for `malloc`.
    let p = unsafe { malloc(total) };
    if !p.is_null() {
        // SAFETY: the new block holds `total` bytes.
        unsafe { core::ptr::write_bytes(p.cast::<u8>(), 0, total) };
    }
    p
}

#[unsafe(no_mangle)]
unsafe extern "C" fn free(p: *mut c_void) {
    // SAFETY: the C contract: `p` came from malloc (checked by the header).
    if unsafe { heap().free(p.cast()) }.is_err() {
        bad(b"free");
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn realloc(p: *mut c_void, size: usize) -> *mut c_void {
    // SAFETY: as for `free`.
    match unsafe { heap().realloc(p.cast(), size, &mut Pages) } {
        Ok(Some(q)) => q.cast(),
        Ok(None) => nomem(),
        Err(_) => bad(b"realloc"),
    }
}

/// `reallocf`: `realloc` that frees the old block when it fails.
#[unsafe(no_mangle)]
unsafe extern "C" fn reallocf(p: *mut c_void, size: usize) -> *mut c_void {
    // SAFETY: as for `realloc`.
    let q = unsafe { realloc(p, size) };
    if q.is_null() && !p.is_null() {
        // SAFETY: `p` is still ours after a failed realloc.
        unsafe { free(p) };
    }
    q
}

#[unsafe(no_mangle)]
unsafe extern "C" fn posix_memalign(out: *mut *mut c_void, align: usize, size: usize) -> i32 {
    if !align.is_power_of_two() || align < core::mem::size_of::<usize>() {
        return EINVAL;
    }
    // SAFETY: as for `malloc`.
    match unsafe { heap().alloc_aligned(size, align, &mut Pages) } {
        Some(p) => {
            // SAFETY: the C contract: `out` points to a pointer.
            unsafe { *out = p.cast() };
            0
        }
        None => ENOMEM,
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn aligned_alloc(align: usize, size: usize) -> *mut c_void {
    if !align.is_power_of_two() {
        crate::set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: as for `malloc`.
    match unsafe { heap().alloc_aligned(size, align, &mut Pages) } {
        Some(p) => p.cast(),
        None => nomem(),
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn valloc(size: usize) -> *mut c_void {
    // SAFETY: as for `malloc`.
    unsafe { aligned_alloc(4096, size) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn malloc_size(p: *const c_void) -> usize {
    // SAFETY: the heap reads nothing outside its arenas; a pointer malloc
    // did not return, or a freed one, gives 0.
    unsafe { heap().usable_size(p as *mut u8) }.unwrap_or(0)
}

/// `malloc_good_size`: the size a request of `size` bytes really gets.
#[unsafe(no_mangle)]
extern "C" fn malloc_good_size(size: usize) -> usize {
    poly_libsystem::heap::good_size(size)
}
