//! The global interface table across apartments: registered on an apartment-threaded thread and fetched on a
//! multithreaded one, the other way round, between two apartment-threaded threads, revoked by another apartment, and
//! fetched after the registering thread has ended.
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

// ---- an object of the test's own (IUnknown only), and the global interface table's three methods ----

use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

#[repr(C)]
struct ObjVtbl {
    qi: extern "system" fn(*mut Obj, *const GUID, *mut *mut c_void) -> u32,
    addref: extern "system" fn(*mut Obj) -> u32,
    release: extern "system" fn(*mut Obj) -> u32,
}

/// Our object: a reference count and counters of what was asked of it.
#[repr(C)]
struct Obj {
    vtbl: *const ObjVtbl,
    refs: AtomicI32,
    qi_unknown: AtomicU32,
    qi_marshal: AtomicU32,
    qi_other: AtomicU32,
}

unsafe impl Sync for Obj {}

extern "system" fn obj_qi(this: *mut Obj, iid: *const GUID, out: *mut *mut c_void) -> u32 {
    // SAFETY: `this` is one of the statics below; `iid` and `out` are the caller's live pointers.
    unsafe {
        let o = &*this;
        let g = &*iid;
        if g.d1 == 0 && g.d4[0] == 0xC0 {
            o.qi_unknown.fetch_add(1, Ordering::SeqCst);
            o.refs.fetch_add(1, Ordering::SeqCst);
            *out = this as *mut c_void;
            return 0;
        }
        if g.d1 == 3 && g.d4[0] == 0xC0 {
            o.qi_marshal.fetch_add(1, Ordering::SeqCst);
        } else {
            o.qi_other.fetch_add(1, Ordering::SeqCst);
        }
        *out = core::ptr::null_mut();
        0x8000_4002
    }
}

extern "system" fn obj_addref(this: *mut Obj) -> u32 {
    // SAFETY: one of the statics below.
    unsafe { (*this).refs.fetch_add(1, Ordering::SeqCst) as u32 + 1 }
}

extern "system" fn obj_release(this: *mut Obj) -> u32 {
    // SAFETY: one of the statics below.
    unsafe { ((*this).refs.fetch_sub(1, Ordering::SeqCst) - 1) as u32 }
}

static OBJ_VTBL: ObjVtbl = ObjVtbl { qi: obj_qi, addref: obj_addref, release: obj_release };

// SAFETY of the raw pointer in a static: it points at a static table that is never written.
unsafe impl Sync for ObjVtbl {}

const fn new_obj() -> Obj {
    Obj {
        vtbl: &OBJ_VTBL,
        refs: AtomicI32::new(1),
        qi_unknown: AtomicU32::new(0),
        qi_marshal: AtomicU32::new(0),
        qi_other: AtomicU32::new(0),
    }
}

static OBJ_A: Obj = new_obj();
static OBJ_B: Obj = new_obj();

fn obj_ptr(o: &'static Obj) -> *mut c_void {
    o as *const Obj as *mut c_void
}

fn refs(o: &Obj) -> i32 {
    o.refs.load(Ordering::SeqCst)
}

/// Our object's reference count as text.
fn obj_state(o: &Obj) -> Buf<100> {
    let mut b = Buf::<100>::new();
    let _ = write!(b, "object: references {}", refs(o));
    b
}

/// An interface pointer's vtable slot `i`.
fn slot_fn(p: *mut c_void, i: usize) -> usize {
    // SAFETY: `p` is a COM interface pointer; every vtable has the slot.
    unsafe { *(*(p as *const *const usize)).add(i) }
}

fn com_release(p: *mut c_void) -> u32 {
    if p.is_null() {
        return 0;
    }
    // SAFETY: slot 2 of every COM vtable is Release.
    unsafe {
        let f: extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(slot_fn(p, 2));
        f(p)
    }
}

fn com_addref(p: *mut c_void) -> u32 {
    // SAFETY: slot 1 of every COM vtable is AddRef.
    unsafe {
        let f: extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(slot_fn(p, 1));
        f(p)
    }
}

fn com_qi(p: *mut c_void, iid: &GUID, out: &mut *mut c_void) -> u32 {
    // SAFETY: slot 0 of every COM vtable is QueryInterface.
    unsafe {
        let f: extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> u32 =
            core::mem::transmute(slot_fn(p, 0));
        f(p, iid, out)
    }
}

/// IGlobalInterfaceTable (objidl.h): slots 3, 4 and 5.
fn git_register(git: *mut c_void, unk: *mut c_void, iid: &GUID, cookie: &mut u32) -> u32 {
    // SAFETY: slot 3 is RegisterInterfaceInGlobal(IUnknown*, REFIID, DWORD*).
    unsafe {
        let f: extern "system" fn(*mut c_void, *mut c_void, *const GUID, *mut u32) -> u32 =
            core::mem::transmute(slot_fn(git, 3));
        f(git, unk, iid, cookie)
    }
}

fn git_revoke(git: *mut c_void, cookie: u32) -> u32 {
    // SAFETY: slot 4 is RevokeInterfaceFromGlobal(DWORD).
    unsafe {
        let f: extern "system" fn(*mut c_void, u32) -> u32 = core::mem::transmute(slot_fn(git, 4));
        f(git, cookie)
    }
}

fn git_get(git: *mut c_void, cookie: u32, iid: &GUID, out: &mut *mut c_void) -> u32 {
    // SAFETY: slot 5 is GetInterfaceFromGlobal(DWORD, REFIID, void**).
    unsafe {
        let f: extern "system" fn(*mut c_void, u32, *const GUID, *mut *mut c_void) -> u32 =
            core::mem::transmute(slot_fn(git, 5));
        f(git, cookie, iid, out)
    }
}

/// CoCreateInstance of the global interface table; the table or NULL.
fn git_create(l: &mut Lines, what: &str, ctx: u32) -> *mut c_void {
    let mut g: *mut c_void = core::ptr::null_mut();
    // SAFETY: live out pointer.
    let hr = unsafe {
        CoCreateInstance(&CLSID_StdGlobalInterfaceTable, core::ptr::null_mut(), ctx, &IID_IGlobalInterfaceTable, &mut g)
    };
    lsay!(
        l,
        "CoCreateInstance(CLSID_StdGlobalInterfaceTable, context {ctx:#x}) {what} = {hr:#x}, interface returned {}",
        u32::from(!g.is_null())
    );
    g
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoWaitForMultipleHandles(flags: u32, ms: u32, n: u32, handles: *const HANDLE, index: *mut u32) -> u32;
}

/// Wait for `h` while the apartment still serves calls from other apartments (an STA thread must do this, not WaitForSingleObject).
fn wait_serving(h: HANDLE) -> bool {
    let mut idx = 0u32;
    // COWAIT_DISPATCH_CALLS 8 | COWAIT_DISPATCH_WINDOW_MESSAGES 4 (combaseapi.h)
    // SAFETY: one live handle.
    unsafe { CoWaitForMultipleHandles(0xC, 10_000, 1, &h, &mut idx) == 0 }
}

static mut EV: [HANDLE; 12] = [core::ptr::null_mut(); 12];
static COOKIE: AtomicU32 = AtomicU32::new(0);
static GIT_PTR: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

fn ev(i: usize) -> HANDLE {
    // SAFETY: set up before the threads start.
    unsafe { (*core::ptr::addr_of!(EV))[i] }
}

fn fetch(l: &mut Lines, who: &str, cookie: u32, o: &'static Obj) -> *mut c_void {
    let git = git_create(l, who, CLSCTX_INPROC_SERVER);
    if git.is_null() {
        return core::ptr::null_mut();
    }
    let mut got: *mut c_void = core::ptr::null_mut();
    let hr = git_get(git, cookie, &IID_IUnknown, &mut got);
    lsay!(
        l,
        "GetInterfaceFromGlobal(cookie) {who} = {hr:#x}, interface returned {}, the very object: {}",
        u32::from(!got.is_null()),
        u32::from(got == obj_ptr(o))
    );
    com_release(git);
    got
}

// -- case 1: an STA thread registers, an MTA thread fetches and revokes

extern "system" fn c1_sta(p: *mut c_void) -> u32 {
    let l = slot(p as usize);
    coinit(l, "apartment threaded (the registering thread)", COINIT_APARTMENTTHREADED);
    let git = git_create(l, "in the STA", CLSCTX_INPROC_SERVER);
    let mut c = 0u32;
    let hr = git_register(git, obj_ptr(&OBJ_A), &IID_IUnknown, &mut c);
    lsay!(l, "RegisterInterfaceInGlobal in the STA = {hr:#x}, cookie not 0: {}", u32::from(c != 0));
    lsay!(l, "after it: {}", obj_state(&OBJ_A).as_str());
    COOKIE.store(c, Ordering::SeqCst);
    GIT_PTR.store(git as usize, Ordering::SeqCst);
    set(ev(0));
    wait_serving(ev(1));
    lsay!(l, "after the other apartment's calls: {}", obj_state(&OBJ_A).as_str());
    // the registration is gone: fetching in the own apartment
    let mut got: *mut c_void = core::ptr::null_mut();
    let hr = git_get(git, c, &IID_IUnknown, &mut got);
    lsay!(
        l,
        "GetInterfaceFromGlobal in the STA after the MTA revoked = {hr:#x}, interface returned {}",
        u32::from(!got.is_null())
    );
    com_release(got);
    com_release(git);
    couninit();
    0
}

extern "system" fn c1_mta(p: *mut c_void) -> u32 {
    let l = slot(p as usize);
    wait(ev(0));
    coinit(l, "multithreaded (the fetching thread)", COINIT_MULTITHREADED);
    let c = COOKIE.load(Ordering::SeqCst);
    let got = fetch(l, "in the MTA", c, &OBJ_A);
    let mut again = core::ptr::null_mut();
    if !got.is_null() {
        let hr = com_qi(got, &IID_IUnknown, &mut again);
        lsay!(l, "QueryInterface(IUnknown) on what the MTA got = {hr:#x}");
        com_release(again);
    }
    com_release(got);
    let git = git_create(l, "in the MTA, for the revoke", CLSCTX_INPROC_SERVER);
    lsay!(
        l,
        "the table object of the MTA is the one the STA created: {}",
        u32::from(git as usize == GIT_PTR.load(Ordering::SeqCst))
    );
    let hr = git_revoke(git, c);
    lsay!(l, "RevokeInterfaceFromGlobal in the MTA, of a registration made by an STA = {hr:#x}");
    com_release(git);
    couninit();
    set(ev(1));
    0
}

// -- case 2: an MTA thread registers, an STA thread fetches

extern "system" fn c2_mta(p: *mut c_void) -> u32 {
    let l = slot(p as usize);
    coinit(l, "multithreaded (the registering thread)", COINIT_MULTITHREADED);
    let git = git_create(l, "in the MTA", CLSCTX_INPROC_SERVER);
    let mut c = 0u32;
    let hr = git_register(git, obj_ptr(&OBJ_B), &IID_IUnknown, &mut c);
    lsay!(l, "RegisterInterfaceInGlobal in the MTA = {hr:#x}, cookie not 0: {}", u32::from(c != 0));
    COOKIE.store(c, Ordering::SeqCst);
    set(ev(2));
    wait(ev(3));
    let hr = git_revoke(git, c);
    lsay!(l, "RevokeInterfaceFromGlobal in the MTA = {hr:#x}");
    lsay!(l, "after it: {}", obj_state(&OBJ_B).as_str());
    com_release(git);
    couninit();
    0
}

extern "system" fn c2_sta(p: *mut c_void) -> u32 {
    let l = slot(p as usize);
    wait(ev(2));
    coinit(l, "apartment threaded (the fetching thread)", COINIT_APARTMENTTHREADED);
    let got = fetch(l, "in the STA", COOKIE.load(Ordering::SeqCst), &OBJ_B);
    com_release(got);
    couninit();
    set(ev(3));
    0
}

// -- case 3: two STA threads; the registering one ends before a third fetches

extern "system" fn c3_reg(p: *mut c_void) -> u32 {
    let l = slot(p as usize);
    coinit(l, "apartment threaded (registers, then ends)", COINIT_APARTMENTTHREADED);
    let git = git_create(l, "in the first STA", CLSCTX_INPROC_SERVER);
    let mut c = 0u32;
    let hr = git_register(git, obj_ptr(&OBJ_A), &IID_IUnknown, &mut c);
    lsay!(l, "RegisterInterfaceInGlobal = {hr:#x}");
    COOKIE.store(c, Ordering::SeqCst);
    com_release(git);
    set(ev(4));
    wait_serving(ev(5)); // a second STA fetches while this one serves calls
    couninit();
    set(ev(6));
    0
}

extern "system" fn c3_other(p: *mut c_void) -> u32 {
    let l = slot(p as usize);
    wait(ev(4));
    coinit(l, "apartment threaded (another STA)", COINIT_APARTMENTTHREADED);
    let got = fetch(l, "in a second STA while the registering STA lives", COOKIE.load(Ordering::SeqCst), &OBJ_A);
    com_release(got);
    set(ev(5));
    wait(ev(6)); // the registering thread has called CoUninitialize
    sleep_ms(200);
    let got = fetch(
        l,
        "in a second STA after the registering STA called CoUninitialize",
        COOKIE.load(Ordering::SeqCst),
        &OBJ_A,
    );
    com_release(got);
    couninit();
    0
}

fn sleep_ms(ms: u32) {
    // SAFETY: plain call.
    unsafe { Sleep(ms) };
}

fn test(t: &mut Test) {
    // SAFETY: before any thread starts.
    unsafe {
        for i in 0..12 {
            (*core::ptr::addr_of_mut!(EV))[i] = event();
        }
    }
    let hs = [spawn(c1_sta, 0), spawn(c1_mta, 1)];
    for h in hs {
        join(h);
    }
    say!(t, "--- registered in an apartment-threaded thread, used in a multithreaded one");
    slot(0).print(t);
    slot(1).print(t);
    let hs = [spawn(c2_mta, 2), spawn(c2_sta, 3)];
    for h in hs {
        join(h);
    }
    say!(t, "--- registered in a multithreaded thread, fetched in an apartment-threaded one");
    slot(2).print(t);
    slot(3).print(t);
    let hs = [spawn(c3_reg, 4), spawn(c3_other, 5)];
    for h in hs {
        join(h);
    }
    say!(t, "--- two apartment-threaded threads; the registering one ends");
    slot(4).print(t);
    slot(5).print(t);
}
