//! Our `user32.dll` (ADR 0087): the two string functions console programs
//! such as 7-Zip's command line import from it, `CharUpperW` and
//! `CharPrevExA`. Windows, messages and input come with Phase 6; until then
//! this is all of `user32`. The decisions are
//! `poly_kernelbase::oleaut`'s; the case mapping is our `ntdll`'s
//! (`RtlUpcaseUnicodeChar`, the object manager's upcase table).

#![no_std]
#![allow(clippy::missing_safety_doc)]

use poly_win32_rt as _;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlUpcaseUnicodeChar(c: u16) -> u16;
}

/// `poly-kernelbase` links `alloc`, so the DLL needs an allocator, though
/// nothing here allocates: every request fails.
struct NoHeap;

// SAFETY: `alloc` returns null (an allocation failure), which is always
// sound; `dealloc` is never reached with a block.
unsafe impl core::alloc::GlobalAlloc for NoHeap {
    unsafe fn alloc(&self, _: core::alloc::Layout) -> *mut u8 {
        core::ptr::null_mut()
    }
    unsafe fn dealloc(&self, _: *mut u8, _: core::alloc::Layout) {}
}

#[global_allocator]
static ALLOC: NoHeap = NoHeap;

fn upcase(c: u16) -> u16 {
    // SAFETY: an ntdll export without preconditions.
    unsafe { RtlUpcaseUnicodeChar(c) }
}

/// `CharUpperW`: Learn: a string is converted in place and returned; a
/// value whose high-order word is zero is a single character, returned
/// converted in the low-order word. Poly maps case without the locale
/// (Learn: "CharUpper always maps lowercase I ("i") to uppercase I").
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CharUpperW(s: *mut u16) -> *mut u16 {
    if poly_kernelbase::oleaut::is_single_char(s as usize) {
        return usize::from(upcase(s as usize as u16)) as *mut u16;
    }
    let mut p = s;
    // SAFETY: a NUL-terminated string the caller owns.
    unsafe {
        while *p != 0 {
            *p = upcase(*p);
            p = p.add(1);
        }
    }
    s
}

/// `CharPrevExA`: the character before `current`, or `start` at the start
/// (Poly's code pages have no lead bytes; `dwFlags` is reserved).
#[unsafe(no_mangle)]
pub extern "system" fn CharPrevExA(_code_page: u16, start: *const u8, current: *const u8, _flags: u32) -> *mut u8 {
    poly_kernelbase::oleaut::char_prev(start as usize, current as usize) as *mut u8
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn user32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
