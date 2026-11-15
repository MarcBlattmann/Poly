//! TEB.ReservedForOle across threads: the main thread, a worker, a thread that never initialised COM while another
//! thread holds the multithreaded apartment (implicit MTA), the first and a later apartment-threaded thread of the
//! process, and what the implicit MTA thread has after it used COM.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all)]

poly_wtest::main!(test);

// The declarations in this file are from Microsoft Learn (prototypes) and the public SDK headers named beside
// them (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;

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

/// One observation line, formatted.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<700>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentThreadId() -> u32;
    fn Sleep(ms: u32);
    fn CreateEventW(sa: *const c_void, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn SetEvent(h: HANDLE) -> BOOL;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn CreateThread(
        sa: *const c_void,
        stack: usize,
        start: extern "system" fn(*mut c_void) -> u32,
        param: *mut c_void,
        flags: u32,
        tid: *mut u32,
    ) -> HANDLE;
    fn GetExitCodeThread(h: HANDLE, code: *mut u32) -> BOOL;
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *const c_void, flags: u32) -> u32;
    fn CoUninitialize();
    fn CoGetApartmentType(kind: *mut i32, qualifier: *mut i32) -> u32;
    fn OleInitialize(reserved: *const c_void) -> u32;
    fn OleUninitialize();
    fn CoCreateInstance(
        clsid: *const GUID,
        outer: *mut c_void,
        ctx: u32,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> u32;
}

/// guiddef.h `GUID`.
#[repr(C)]
#[derive(Clone, Copy)]
struct GUID {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> GUID {
    GUID { d1, d2, d3, d4 }
}

const COINIT_MULTITHREADED: u32 = 0;
const COINIT_APARTMENTTHREADED: u32 = 2;
const COINIT_DISABLE_OLE1DDE: u32 = 4;
const COINIT_SPEED_OVER_MEMORY: u32 = 8;

/// `TEB.ReservedForOle` (winternl.h): offset 0x1758 of the 64-bit TEB (`gs:[0x30]` is its own address), 0xF80 of the 32-bit one.
fn ole_block() -> *const u8 {
    let p: *const u8;
    #[cfg(target_arch = "x86_64")]
    // SAFETY: GS points at the current thread's TEB (winnt.h NtCurrentTeb); the field is part of it (winternl.h layout).
    unsafe {
        let teb: *const u8;
        core::arch::asm!("mov {}, gs:[0x30]", out(reg) teb, options(nostack, readonly, preserves_flags));
        p = *(teb.add(0x1758) as *const *const u8);
    }
    #[cfg(target_arch = "x86")]
    // SAFETY: FS points at the current thread's TEB (winnt.h NtCurrentTeb); the field is part of it (winternl.h layout).
    unsafe {
        let teb: *const u8;
        core::arch::asm!("mov {}, fs:[0x18]", out(reg) teb, options(nostack, readonly, preserves_flags));
        p = *(teb.add(0xF80) as *const *const u8);
    }
    p
}

/// The 32-bit value at byte offset `off` of the block `p` points to.
fn dword_at(p: *const u8, off: usize) -> u32 {
    // SAFETY: `p` is what TEB.ReservedForOle holds, a live block of the thread's COM state; the offsets read are within its first 0x80 bytes.
    unsafe { core::ptr::read_volatile(p.add(off) as *const u32) }
}

/// What `CoGetApartmentType` says, as text.
fn apt_text() -> Buf<80> {
    let mut kind = 99i32;
    let mut qual = 99i32;
    // SAFETY: live out pointers.
    let hr = unsafe { CoGetApartmentType(&mut kind, &mut qual) };
    let mut b = Buf::<80>::new();
    if hr == 0 {
        let _ = write!(b, "CoGetApartmentType 0, type {kind}, qualifier {qual}");
    } else {
        let _ = write!(b, "CoGetApartmentType {hr:#x} (outputs untouched {})", u32::from(kind == 99 && qual == 99));
    }
    b
}

/// A wait on a manual-reset event, 10 s at most; true when it was set.
fn wait(h: HANDLE) -> bool {
    // SAFETY: a handle this test created.
    unsafe { WaitForSingleObject(h, 10_000) == 0 }
}

fn event() -> HANDLE {
    // SAFETY: an unnamed manual-reset event.
    unsafe { CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()) }
}

fn set(h: HANDLE) {
    // SAFETY: a handle this test created.
    unsafe { SetEvent(h) };
}

fn close(h: HANDLE) {
    if !h.is_null() && h != INVALID_HANDLE_VALUE {
        // SAFETY: a handle this test created, closed once.
        unsafe { CloseHandle(h) };
    }
}

/// Where a thread's lines go: the parent prints them after the thread has ended.
struct Lines {
    b: Buf<6000>,
}

impl Lines {
    const fn new() -> Lines {
        Lines { b: Buf::new() }
    }
    fn add(&mut self, s: &str) {
        let _ = write!(self.b, "{s}\n");
    }
    fn print(&self, t: &mut Test) {
        for l in self.b.as_str().split('\n') {
            if !l.is_empty() {
                t.line(l);
            }
        }
    }
}

macro_rules! lsay {
    ($l:expr, $($a:tt)*) => {{
        let mut b = Buf::<700>::new();
        let _ = write!(b, $($a)*);
        $l.add(b.as_str());
    }};
}

/// Run `f(param)` on a new thread and wait for it (30 s at most); its exit code.
fn spawn(f: extern "system" fn(*mut c_void) -> u32, param: usize) -> HANDLE {
    let mut tid = 0u32;
    // SAFETY: a thread that ends by itself; the caller waits for it and closes the handle.
    unsafe { CreateThread(core::ptr::null(), 0, f, param as *mut c_void, 0, &mut tid) }
}

fn join(h: HANDLE) -> u32 {
    let mut code = 0xFFFFu32;
    // SAFETY: the handle of a thread this test started.
    unsafe {
        WaitForSingleObject(h, 30_000);
        GetExitCodeThread(h, &mut code);
    }
    close(h);
    code
}

/// `TEB.ReservedForOle` and what is behind it, as one line: only NULL or not, and the plain values at +0x14, +0x28 and +0x2c of the block.
fn snap(l: &mut Lines, label: &str) -> *const u8 {
    let p = ole_block();
    let mut b = Buf::<400>::new();
    if p.is_null() {
        let _ = write!(b, "{label}: ReservedForOle NULL; {}", apt_text().as_str());
    } else {
        let _ = write!(
            b,
            "{label}: ReservedForOle set, +0x14 {:#x}, +0x28 {}, +0x2c {}; {}",
            dword_at(p, 0x14),
            dword_at(p, 0x28),
            dword_at(p, 0x2c),
            apt_text().as_str()
        );
    }
    l.add(b.as_str());
    p
}

fn same(l: &mut Lines, what: &str, a: *const u8, b: *const u8) {
    lsay!(l, "{what}: {}", if a == b { "same block" } else { "another block" });
}

fn coinit(l: &mut Lines, what: &str, f: u32) -> u32 {
    // SAFETY: plain call.
    let hr = unsafe { CoInitializeEx(core::ptr::null(), f) };
    lsay!(l, "CoInitializeEx({f:#x}) {what} = {hr:#x}");
    hr
}

fn couninit() {
    // SAFETY: plain call.
    unsafe { CoUninitialize() };
}

/// The slots threads write their lines into (the parent prints them in order after the join).
static mut OUT: [Lines; 12] = [const { Lines::new() }; 12];

fn slot(k: usize) -> &'static mut Lines {
    // SAFETY: each thread writes only its own slot; the parent reads after the join.
    unsafe { &mut (*core::ptr::addr_of_mut!(OUT))[k] }
}

/// Start thread `k` (it writes into `slot(k)`), wait for it, print what it wrote under `title`.
fn run_slot(t: &mut Test, f: extern "system" fn(*mut c_void) -> u32, k: usize, title: &str) {
    let h = spawn(f, k);
    let code = join(h);
    say!(t, "--- {title}{}", if code == 0 { "" } else { " (thread did not end normally)" });
    slot(k).print(t);
}

/// UTF-16 for an ASCII literal, NUL-terminated.
fn wide<const N: usize>(s: &str) -> [u16; N] {
    let mut w = [0u16; N];
    for (i, c) in s.bytes().enumerate().take(N - 1) {
        w[i] = u16::from(c);
    }
    w
}

/// A function of a DLL that is not imported by name (looked up at run time); NULL when it is not there.
fn proc_of(dll: &str, name: &[u8]) -> *mut c_void {
    let w = wide::<40>(dll);
    // SAFETY: NUL-terminated names.
    unsafe {
        let m = LoadLibraryW(w.as_ptr());
        if m.is_null() {
            return core::ptr::null_mut();
        }
        GetProcAddress(m, name.as_ptr())
    }
}

const CLSID_StdGlobalInterfaceTable: GUID = guid(0x0000_0323, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IUnknown: GUID = guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IMarshal: GUID = guid(3, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IGlobalInterfaceTable: GUID = guid(0x0000_0146, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const CLSID_BackgroundCopyManager: GUID =
    guid(0x4991_d34b, 0x80a1, 0x4291, [0x83, 0xb6, 0x33, 0x28, 0x36, 0x6b, 0x90, 0x97]);
const IID_IBackgroundCopyManager: GUID =
    guid(0x5ce3_4c0d, 0x0dc9, 0x4c1f, [0x89, 0x7c, 0xda, 0xa1, 0xb7, 0x8c, 0xee, 0x7c]);
const CLSCTX_INPROC_SERVER: u32 = 1;
const CLSCTX_LOCAL_SERVER: u32 = 4;
const CLSCTX_REMOTE_SERVER: u32 = 0x10;
const CLSCTX_ALL: u32 = 0x17;

static mut EV: [HANDLE; 6] = [core::ptr::null_mut(); 6];

fn ev(i: usize) -> HANDLE {
    // SAFETY: set up before the threads start.
    unsafe { (*core::ptr::addr_of!(EV))[i] }
}

/// Holds the multithreaded apartment until event 1 is set.
extern "system" fn holder(p: *mut c_void) -> u32 {
    let l = slot(p as usize);
    coinit(l, "multithreaded (the holder)", COINIT_MULTITHREADED);
    snap(l, "holder after it");
    set(ev(0));
    wait(ev(1));
    couninit();
    snap(l, "holder after its CoUninitialize");
    0
}

/// Never calls CoInitializeEx; looks while the holder is in the MTA.
extern "system" fn implicit(p: *mut c_void) -> u32 {
    let l = slot(p as usize);
    wait(ev(0));
    snap(l, "implicit MTA thread, before any call");
    let mut o: *mut c_void = core::ptr::null_mut();
    // SAFETY: live out pointer.
    let hr = unsafe {
        CoCreateInstance(
            &CLSID_StdGlobalInterfaceTable,
            core::ptr::null_mut(),
            CLSCTX_ALL,
            &IID_IGlobalInterfaceTable,
            &mut o,
        )
    };
    lsay!(l, "CoCreateInstance(the global interface table) = {hr:#x}, object returned {}", u32::from(!o.is_null()));
    snap(l, "implicit MTA thread, after creating an object");
    release(o);
    set(ev(1));
    0
}

fn release(o: *mut c_void) {
    if o.is_null() {
        return;
    }
    // SAFETY: an interface pointer we own; slot 2 of every COM vtable is Release.
    unsafe {
        let vt = *(o as *const *const usize);
        let f: extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vt.add(2));
        f(o);
    }
}

/// A thread that is the first apartment-threaded one: initialises, tells event 2, waits for event 3.
extern "system" fn first_sta(p: *mut c_void) -> u32 {
    let l = slot(p as usize);
    coinit(l, "apartment threaded (the first of the process)", COINIT_APARTMENTTHREADED);
    snap(l, "first STA thread");
    set(ev(2));
    wait(ev(3));
    couninit();
    0
}

extern "system" fn second_sta(p: *mut c_void) -> u32 {
    let l = slot(p as usize);
    wait(ev(2));
    coinit(
        l,
        "apartment threaded with DISABLE_OLE1DDE (a later one)",
        COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE,
    );
    snap(l, "later STA thread");
    couninit();
    snap(l, "later STA thread after CoUninitialize");
    set(ev(3));
    0
}

/// A thread that is in no apartment, with nobody else initialised.
extern "system" fn alone(p: *mut c_void) -> u32 {
    let l = slot(p as usize);
    snap(l, "no apartment anywhere");
    0
}

fn test(t: &mut Test) {
    // SAFETY: before any thread starts.
    unsafe {
        for i in 0..6 {
            (*core::ptr::addr_of_mut!(EV))[i] = event();
        }
    }
    let mut l = Lines::new();
    snap(&mut l, "the main thread before any COM call");
    l.print(t);
    run_slot(t, alone, 0, "a worker thread while no thread of the process initialised COM");
    // the implicit MTA
    let hs = [spawn(holder, 1), spawn(implicit, 2)];
    for h in hs {
        join(h);
    }
    say!(t, "--- the holder of the multithreaded apartment");
    slot(1).print(t);
    say!(t, "--- a thread that never initialised, in the process's multithreaded apartment");
    slot(2).print(t);
    // two apartment-threaded threads
    let hs = [spawn(first_sta, 3), spawn(second_sta, 4)];
    for h in hs {
        join(h);
    }
    say!(t, "--- the first apartment-threaded thread of the process");
    slot(3).print(t);
    say!(t, "--- a later apartment-threaded thread while the first one lives");
    slot(4).print(t);
}
