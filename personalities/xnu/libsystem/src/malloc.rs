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
    // SAFETY: as for `free`; a pointer malloc did not return gives 0.
    unsafe { heap().usable_size(p as *mut u8) }.unwrap_or(0)
}

/// `malloc_good_size`: the size a request of `size` bytes really gets.
#[unsafe(no_mangle)]
extern "C" fn malloc_good_size(size: usize) -> usize {
    size.saturating_add(16).max(32).checked_next_power_of_two().map_or(size, |b| b - 16)
}

// ------------------------------------------------------------- zones

/// `malloc_zone_t` (`<malloc/malloc.h>`): the fields up to
/// `pressure_relief`. Poly has one zone, the default one; every zone
/// function is the plain allocator. Programs may call through the
/// function pointers (SQLite calls `size`).
#[repr(C)]
pub struct Zone {
    reserved: [usize; 2],
    size: unsafe extern "C" fn(*mut Zone, *const c_void) -> usize,
    malloc: unsafe extern "C" fn(*mut Zone, usize) -> *mut c_void,
    calloc: unsafe extern "C" fn(*mut Zone, usize, usize) -> *mut c_void,
    valloc: unsafe extern "C" fn(*mut Zone, usize) -> *mut c_void,
    free: unsafe extern "C" fn(*mut Zone, *mut c_void),
    realloc: unsafe extern "C" fn(*mut Zone, *mut c_void, usize) -> *mut c_void,
    destroy: unsafe extern "C" fn(*mut Zone),
    zone_name: *const u8,
    batch_malloc: usize,
    batch_free: usize,
    introspect: usize,
    version: u32,
    memalign: usize,
    free_definite_size: usize,
    pressure_relief: usize,
}

unsafe extern "C" fn zone_size(_: *mut Zone, p: *const c_void) -> usize {
    // SAFETY: the zone contract: a block of this zone (or any pointer:
    // malloc_size answers 0 for one it did not hand out).
    unsafe { malloc_size(p) }
}

unsafe extern "C" fn zone_malloc(_: *mut Zone, n: usize) -> *mut c_void {
    // SAFETY: as malloc.
    unsafe { malloc(n) }
}

unsafe extern "C" fn zone_calloc(_: *mut Zone, n: usize, size: usize) -> *mut c_void {
    // SAFETY: as calloc.
    unsafe { calloc(n, size) }
}

unsafe extern "C" fn zone_valloc(_: *mut Zone, n: usize) -> *mut c_void {
    // SAFETY: as valloc.
    unsafe { valloc(n) }
}

unsafe extern "C" fn zone_free(_: *mut Zone, p: *mut c_void) {
    // SAFETY: as free.
    unsafe { free(p) }
}

unsafe extern "C" fn zone_realloc(_: *mut Zone, p: *mut c_void, n: usize) -> *mut c_void {
    // SAFETY: as realloc.
    unsafe { realloc(p, n) }
}

unsafe extern "C" fn zone_destroy(_: *mut Zone) {}

static DEFAULT_ZONE: crate::Global<Zone> = crate::Global::new(Zone {
    reserved: [0; 2],
    size: zone_size,
    malloc: zone_malloc,
    calloc: zone_calloc,
    valloc: zone_valloc,
    free: zone_free,
    realloc: zone_realloc,
    destroy: zone_destroy,
    zone_name: c"DefaultMallocZone".as_ptr() as *const u8,
    batch_malloc: 0,
    batch_free: 0,
    introspect: 0,
    version: 0,
    memalign: 0,
    free_definite_size: 0,
    pressure_relief: 0,
});

fn default_zone() -> *mut Zone {
    DEFAULT_ZONE.0.get()
}

#[unsafe(no_mangle)]
extern "C" fn malloc_default_zone() -> *mut Zone {
    default_zone()
}

/// `malloc_create_zone`: Poly's zones are all the default one.
#[unsafe(no_mangle)]
extern "C" fn malloc_create_zone(_start_size: usize, _flags: u32) -> *mut Zone {
    default_zone()
}

#[unsafe(no_mangle)]
extern "C" fn malloc_destroy_zone(_z: *mut Zone) {}

#[unsafe(no_mangle)]
extern "C" fn malloc_set_zone_name(_z: *mut Zone, _name: *const u8) {}

#[unsafe(no_mangle)]
extern "C" fn malloc_zone_from_ptr(_p: *const c_void) -> *mut Zone {
    default_zone()
}

#[unsafe(no_mangle)]
unsafe extern "C" fn malloc_zone_malloc(_z: *mut Zone, n: usize) -> *mut c_void {
    // SAFETY: as malloc.
    unsafe { malloc(n) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn malloc_zone_calloc(_z: *mut Zone, n: usize, size: usize) -> *mut c_void {
    // SAFETY: as calloc.
    unsafe { calloc(n, size) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn malloc_zone_realloc(_z: *mut Zone, p: *mut c_void, n: usize) -> *mut c_void {
    // SAFETY: as realloc.
    unsafe { realloc(p, n) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn malloc_zone_free(_z: *mut Zone, p: *mut c_void) {
    // SAFETY: as free.
    unsafe { free(p) }
}
