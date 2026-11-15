//! CoTaskMemAlloc and CoTaskMemFree (and CoTaskMemRealloc): the block is not
//! NULL for sizes from 0 up; it is 16-byte aligned (as the 64-bit heap
//! allocator does), writable over its whole size and not overlapping others;
//! a size of 0 gives a block that can be freed; huge sizes fail with NULL;
//! free of NULL does nothing; the memory works without COM being initialized
//! and from any thread (allocated on one, freed on another); realloc keeps the
//! contents and grows, shrinks and frees (size 0 with a block). PropVariantClear
//! and StringFromCLSID rely on this being the allocator (see their sheets).
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
type THREADPROC = unsafe extern "system" fn(*mut c_void) -> u32;
/// A fixed-size text buffer for formatting lines.
struct Buf<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Buf<N> {
    const fn new() -> Self {
        Buf { b: [0; N], n: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < N {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

/// `t.line` with formatting.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<300>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// Run `f(arg)` on a new thread and wait for it.
fn on_thread(f: unsafe extern "system" fn(*mut c_void) -> u32, arg: *mut c_void) -> u32 {
    // SAFETY: the thread is joined before returning; `arg` outlives it.
    unsafe {
        let h = CreateThread(core::ptr::null_mut(), 0, f, arg, 0, core::ptr::null_mut());
        WaitForSingleObject(h, 20_000);
        let mut code = 0xFFFF_FFFFu32;
        GetExitCodeThread(h, &mut code);
        CloseHandle(h);
        code
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateThread(
        attributes: *mut c_void,
        stack: usize,
        start: THREADPROC,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
    fn WaitForSingleObject(handle: HANDLE, ms: u32) -> u32;
    fn CloseHandle(handle: HANDLE) -> BOOL;
    fn GetExitCodeThread(thread: HANDLE, code: *mut u32) -> BOOL;
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoTaskMemAlloc(size: usize) -> *mut c_void;
    fn CoTaskMemFree(p: *mut c_void);
    fn CoTaskMemRealloc(p: *mut c_void, size: usize) -> *mut c_void;
}

poly_wtest::main!(test);

unsafe extern "system" fn freer(p: *mut c_void) -> u32 {
    // SAFETY: the block was allocated by the main thread and is freed once, here.
    unsafe {
        let b = *(p as *mut usize) as *mut u8;
        let ok = *b == 0xAB && *b.add(99) == 0xCD;
        CoTaskMemFree(b as *mut c_void);
        ok as u32
    }
}

fn test(t: &mut Test) {
    // SAFETY: blocks from CoTaskMemAlloc used within their size.
    unsafe {
        for size in [0usize, 1, 7, 8, 15, 16, 17, 100, 4096, 65536, 1 << 20] {
            let p = CoTaskMemAlloc(size) as *mut u8;
            let mut ok = !p.is_null();
            if ok && size > 0 {
                core::ptr::write_bytes(p, 0x5A, size);
                ok = *p == 0x5A && *p.add(size - 1) == 0x5A;
            }
            say!(t, "size {size}: not NULL {}, 16-byte aligned {}, writable {ok}", !p.is_null(), p as usize % 16 == 0);
            CoTaskMemFree(p as *mut c_void);
        }
        // blocks do not overlap
        let a = CoTaskMemAlloc(100) as *mut u8;
        let b = CoTaskMemAlloc(100) as *mut u8;
        core::ptr::write_bytes(a, 0x11, 100);
        core::ptr::write_bytes(b, 0x22, 100);
        let mut same = true;
        for i in 0..100 {
            same &= *a.add(i) == 0x11 && *b.add(i) == 0x22;
        }
        say!(t, "two blocks of 100 bytes: different {}, contents kept apart {same}", a != b);
        CoTaskMemFree(a as *mut c_void);
        CoTaskMemFree(b as *mut c_void);
        // huge
        let h = CoTaskMemAlloc(usize::MAX / 2);
        say!(t, "an allocation of half the address space: {}", if h.is_null() { "NULL" } else { "not NULL" });
        let h = CoTaskMemAlloc(usize::MAX);
        say!(t, "an allocation of usize::MAX: {}", if h.is_null() { "NULL" } else { "not NULL" });
        // free NULL
        CoTaskMemFree(core::ptr::null_mut());
        say!(t, "CoTaskMemFree(NULL) returned");
        // free on another thread
        let c = CoTaskMemAlloc(100) as *mut u8;
        *c = 0xAB;
        *c.add(99) = 0xCD;
        let mut cell = c as usize;
        let ok = on_thread(freer, &mut cell as *mut usize as *mut c_void);
        say!(t, "a block freed by another thread: contents seen there {}", ok);
        // realloc
        let r = CoTaskMemAlloc(10) as *mut u8;
        for i in 0..10 {
            *r.add(i) = i as u8;
        }
        let r2 = CoTaskMemRealloc(r as *mut c_void, 5000) as *mut u8;
        let mut keep = true;
        for i in 0..10 {
            keep &= *r2.add(i) == i as u8;
        }
        say!(t, "realloc to 5000 bytes keeps the first ten bytes: {keep}");
        let r3 = CoTaskMemRealloc(r2 as *mut c_void, 4) as *mut u8;
        let mut keep = true;
        for i in 0..4 {
            keep &= *r3.add(i) == i as u8;
        }
        say!(t, "realloc down to 4 bytes keeps four bytes: {keep}");
        let r4 = CoTaskMemRealloc(r3 as *mut c_void, 0);
        say!(t, "realloc to size 0 frees the block and gives NULL: {}", r4.is_null());
        let r5 = CoTaskMemRealloc(core::ptr::null_mut(), 32) as *mut u8;
        say!(t, "realloc of NULL allocates: {}", !r5.is_null());
        CoTaskMemFree(r5 as *mut c_void);
    }
}
