//! `RtlGetUnloadEventTraceEx` and `RtlGetUnloadEventTrace`: the array of the last libraries this
//! process unloaded (`poly_rtl::unload`). The loader's unload notification records into it; a crash
//! reporter reads it through the pointers the export hands out (Learn: "RtlGetUnloadEventTraceEx").

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_ldr::ModuleInfo;
use poly_rtl::unload::{ELEMENT_COUNT, ELEMENT_SIZE, Trace};

struct Shared {
    flag: AtomicBool,
    trace: UnsafeCell<Trace>,
}

// SAFETY: writers take `flag`; readers get raw pointers, as the Windows export gives them.
unsafe impl Sync for Shared {}

static TRACE: Shared = Shared { flag: AtomicBool::new(false), trace: UnsafeCell::new(Trace::new()) };
static ELEMENT_SIZE_CELL: u32 = ELEMENT_SIZE;
static ELEMENT_COUNT_CELL: u32 = ELEMENT_COUNT as u32;

/// `TimeDateStamp` and `CheckSum` of the PE header of the image at `base` (still mapped when the
/// unload is announced); 0 for both when the header is not what it should be.
fn header_words(base: u64) -> (u32, u32) {
    let at = |off: u64| base.wrapping_add(off) as *const u8;
    // SAFETY: the module is mapped in this process and its header is the first page.
    unsafe {
        if at(0).cast::<u16>().read_unaligned() != 0x5A4D {
            return (0, 0);
        }
        let nt = u64::from(at(0x3C).cast::<u32>().read_unaligned());
        if nt > 0x1000 || at(nt).cast::<u32>().read_unaligned() != 0x0000_4550 {
            return (0, 0);
        }
        // FileHeader.TimeDateStamp at +8; OptionalHeader.CheckSum at +24 + 64 in both PE32 and PE32+.
        (at(nt + 8).cast::<u32>().read_unaligned(), at(nt + 24 + 64).cast::<u32>().read_unaligned())
    }
}

/// Record that `m` is being unloaded.
pub fn record(m: &ModuleInfo) {
    let (stamp, sum) = header_words(m.base);
    while TRACE.flag.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    // SAFETY: `flag` makes this the only writer.
    unsafe { (*TRACE.trace.get()).record(m.base, m.size, &m.name, stamp, sum) };
    TRACE.flag.store(false, Ordering::Release);
}

/// `RtlGetUnloadEventTraceEx(ElementSize, ElementCount, EventTrace)`: three out pointers; the
/// size of one element, the number of elements, and the array.
///
/// # Safety
/// The three pointers are NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlGetUnloadEventTraceEx(
    element_size: *mut *const u32,
    element_count: *mut *const u32,
    event_trace: *mut *mut u8,
) {
    // SAFETY: the caller's out pointers when non-null.
    unsafe {
        if !element_size.is_null() {
            *element_size = &ELEMENT_SIZE_CELL;
        }
        if !element_count.is_null() {
            *element_count = &ELEMENT_COUNT_CELL;
        }
        if !event_trace.is_null() {
            *event_trace = TRACE.trace.get().cast::<u8>();
        }
    }
}

/// `RtlGetUnloadEventTrace()`: the array.
#[unsafe(no_mangle)]
pub extern "system" fn RtlGetUnloadEventTrace() -> *mut u8 {
    TRACE.trace.get().cast::<u8>()
}
