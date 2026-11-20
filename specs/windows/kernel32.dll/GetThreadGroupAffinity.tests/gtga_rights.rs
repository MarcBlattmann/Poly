//! GetThreadGroupAffinity and the rights of the thread handle: THREAD_QUERY_LIMITED_INFORMATION or
//! THREAD_QUERY_INFORMATION is needed; every other right alone is not enough (ERROR_ACCESS_DENIED).
#![no_std]
#![no_main]
#![allow(
    dead_code,
    non_camel_case_types,
    non_upper_case_globals,
    non_snake_case,
    unused_imports,
    unused_mut,
    clippy::all
)]

// The declarations in this file are from Microsoft Learn (prototypes) and the public SDK headers
// (constants, layouts); the helpers are this test's own. One image, two roles: started with
// `--child <kind> <event name>` it is the TARGET process of the test (it signals the named event, then waits
// or exits); otherwise it is the test, which starts targets of its own image and creates threads in them.

use core::ffi::c_void;
use core::fmt::Write as _;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_wtest::*;

type BOOL = i32;
type HANDLE = *mut c_void;
type DWORD = u32;
type NTSTATUS = i32;

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

/// One observation line, formatted (at most 250 characters survive).
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<250>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// A NUL-terminated UTF-16 copy of a string (up to 299 units).
fn wide(s: &str) -> [u16; 300] {
    let mut w = [0u16; 300];
    for (i, c) in s.chars().enumerate().take(299) {
        w[i] = c as u32 as u16;
    }
    w
}

/// The units of a NUL-terminated UTF-16 string at `p` (at most `max`).
fn wstr_at<'a>(p: *const u16, max: usize) -> &'a [u16] {
    if p.is_null() {
        return &[];
    }
    let mut n = 0;
    // SAFETY: the caller passes a NUL-terminated string the system returned; the scan stops at `max`.
    unsafe {
        while n < max && *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    }
}

#[repr(C)]
struct StartupInfoW {
    cb: u32,
    pad0: u32,
    rest: [usize; 12],
}

#[repr(C)]
struct ProcInfo {
    process: HANDLE,
    thread: HANDLE,
    pid: u32,
    tid: u32,
}

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: *mut c_void,
    inherit: BOOL,
}

#[repr(C)]
struct MemInfo {
    base: usize,
    alloc_base: usize,
    alloc_protect: u32,
    partition: u16,
    pad: u16,
    size: usize,
    state: u32,
    protect: u32,
    kind: u32,
    pad2: u32,
}

#[repr(C)]
struct ThreadBasic {
    exit_status: i32,
    pad: u32,
    teb: usize,
    pid: usize,
    tid: usize,
    affinity: usize,
    priority: i32,
    base_priority: i32,
}

#[repr(C)]
struct ObjBasic {
    attributes: u32,
    granted: u32,
    handle_count: u32,
    pointer_count: u32,
    rest: [u32; 10],
}

const WAIT_TIMEOUT: u32 = 0x102;
const WAIT_FAILED: u32 = 0xFFFF_FFFF;
const INFINITE: u32 = 0xFFFF_FFFF;
const STILL_ACTIVE: u32 = 259;
const CREATE_SUSPENDED: u32 = 4;
const STACK_SIZE_PARAM_IS_A_RESERVATION: u32 = 0x10000;

const PROCESS_TERMINATE: u32 = 0x1;
const PROCESS_CREATE_THREAD: u32 = 0x2;
const PROCESS_VM_OPERATION: u32 = 0x8;
const PROCESS_VM_READ: u32 = 0x10;
const PROCESS_VM_WRITE: u32 = 0x20;
const PROCESS_DUP_HANDLE: u32 = 0x40;
const PROCESS_QUERY_INFORMATION: u32 = 0x400;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const SYNCHRONIZE_: u32 = 0x0010_0000;
const PROCESS_ALL_ACCESS: u32 = 0x1F_FFFF;
const THREAD_ALL_ACCESS: u32 = 0x1F_FFFF;
const MEM_COMMIT_RESERVE: u32 = 0x3000;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThread() -> HANDLE;
    fn GetCurrentThreadId() -> u32;
    fn GetModuleFileNameW(module: HANDLE, buf: *mut u16, size: u32) -> u32;
    fn GetModuleHandleW(name: *const u16) -> HANDLE;
    fn GetProcAddress(m: HANDLE, name: *const u8) -> usize;
    fn GetCommandLineW() -> *const u16;
    fn CreateProcessW(
        app: *const u16,
        cmd: *mut u16,
        pa: *const c_void,
        ta: *const c_void,
        inherit: BOOL,
        flags: u32,
        env: *const c_void,
        dir: *const u16,
        si: *mut StartupInfoW,
        pi: *mut ProcInfo,
    ) -> BOOL;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn GetExitCodeProcess(h: HANDLE, code: *mut u32) -> BOOL;
    fn GetExitCodeThread(h: HANDLE, code: *mut u32) -> BOOL;
    fn TerminateProcess(h: HANDLE, code: u32) -> BOOL;
    fn ResumeThread(h: HANDLE) -> u32;
    fn SuspendThread(h: HANDLE) -> u32;
    fn CreateEventW(sa: *const c_void, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn OpenEventW(access: u32, inherit: BOOL, name: *const u16) -> HANDLE;
    fn SetEvent(h: HANDLE) -> BOOL;
    fn Sleep(ms: u32);
    fn SetErrorMode(mode: u32) -> u32;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn OpenThread(access: u32, inherit: BOOL, tid: u32) -> HANDLE;
    fn DuplicateHandle(
        sp: HANDLE,
        sh: HANDLE,
        tp: HANDLE,
        th: *mut HANDLE,
        access: u32,
        inherit: BOOL,
        opts: u32,
    ) -> BOOL;
    fn GetHandleInformation(h: HANDLE, flags: *mut u32) -> BOOL;
    fn GetThreadId(h: HANDLE) -> u32;
    fn GetProcessIdOfThread(h: HANDLE) -> u32;
    fn GetProcessId(h: HANDLE) -> u32;
    fn CreateThread(sa: *const c_void, stack: usize, start: usize, arg: usize, flags: u32, tid: *mut u32) -> HANDLE;
    fn CreateRemoteThread(
        process: HANDLE,
        sa: *const c_void,
        stack: usize,
        start: usize,
        arg: usize,
        flags: u32,
        tid: *mut u32,
    ) -> HANDLE;
    fn CreateRemoteThreadEx(
        process: HANDLE,
        sa: *const c_void,
        stack: usize,
        start: usize,
        arg: usize,
        flags: u32,
        attrs: *mut c_void,
        tid: *mut u32,
    ) -> HANDLE;
    fn VirtualAllocEx(process: HANDLE, addr: usize, size: usize, kind: u32, prot: u32) -> usize;
    fn VirtualFreeEx(process: HANDLE, addr: usize, size: usize, kind: u32) -> BOOL;
    fn VirtualQueryEx(process: HANDLE, addr: usize, info: *mut MemInfo, len: usize) -> usize;
    fn WriteProcessMemory(process: HANDLE, addr: usize, buf: *const u8, n: usize, written: *mut usize) -> BOOL;
    fn ReadProcessMemory(process: HANDLE, addr: usize, buf: *mut u8, n: usize, read: *mut usize) -> BOOL;
    fn InitializeProcThreadAttributeList(list: *mut c_void, count: u32, flags: u32, size: *mut usize) -> BOOL;
    fn UpdateProcThreadAttribute(
        list: *mut c_void,
        flags: u32,
        attr: usize,
        value: *const c_void,
        size: usize,
        prev: *mut c_void,
        ret: *mut usize,
    ) -> BOOL;
    fn DeleteProcThreadAttributeList(list: *mut c_void);
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlGetLastNtStatus() -> i32;
    fn NtQueryObject(h: HANDLE, class: u32, buf: *mut c_void, len: u32, ret: *mut u32) -> i32;
    fn NtQueryInformationThread(h: HANDLE, class: u32, buf: *mut c_void, len: u32, ret: *mut u32) -> i32;
    fn NtCreateThreadEx(
        thread: *mut HANDLE,
        access: u32,
        oa: *const c_void,
        process: HANDLE,
        start: usize,
        arg: usize,
        flags: u32,
        zero_bits: usize,
        stack_size: usize,
        max_stack: usize,
        attrs: *mut c_void,
    ) -> i32;
}

fn gle() -> i64 {
    // SAFETY: plain call.
    i64::from(unsafe { GetLastError() })
}

fn status_now() -> u32 {
    // SAFETY: plain call.
    unsafe { RtlGetLastNtStatus() as u32 }
}

fn nul() -> HANDLE {
    core::ptr::null_mut()
}

fn is_valid(h: HANDLE) -> bool {
    !h.is_null() && h as isize != -1
}

fn close(h: HANDLE) {
    if is_valid(h) {
        // SAFETY: a handle this test owns.
        unsafe { CloseHandle(h) };
    }
}

fn k32(name: &str) -> usize {
    let w = wide("kernel32.dll");
    // SAFETY: valid names.
    unsafe {
        let m = GetModuleHandleW(w.as_ptr());
        let mut n = [0u8; 64];
        n[..name.len()].copy_from_slice(name.as_bytes());
        GetProcAddress(m, n.as_ptr())
    }
}

fn self_path() -> ([u16; 300], usize) {
    let mut p = [0u16; 300];
    // SAFETY: valid buffer.
    let n = unsafe { GetModuleFileNameW(nul(), p.as_mut_ptr(), 299) } as usize;
    (p, n)
}

/// The text after ` --child ` on this process's command line.
fn child_args() -> Option<Buf<160>> {
    // SAFETY: the command line is a NUL-terminated string owned by the process.
    let cl = wstr_at(unsafe { GetCommandLineW() }, 600);
    let key = b" --child ";
    let mut i = 0;
    while i + key.len() <= cl.len() {
        if key.iter().enumerate().all(|(k, &c)| cl[i + k] == u16::from(c)) {
            let mut b = Buf::<160>::new();
            for &c in &cl[i + key.len()..] {
                let _ = write!(b, "{}", (c as u8) as char);
            }
            return Some(b);
        }
        i += 1;
    }
    None
}

// ---------------------------------------------------------------- targets

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Child {
    process: HANDLE,
    thread: HANDLE,
    ev: HANDLE,
    pid: u32,
    tid: u32,
}

/// Start a target process (this image as a child) of kind `kind` ("sleep": signals ready, then sleeps for ever;
/// "exit": signals ready and exits with code 7). `flags`: creation flags (CREATE_SUSPENDED...).
fn spawn(kind: &str, flags: u32) -> Child {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    // SAFETY: plain call.
    let me = unsafe { GetCurrentProcessId() };
    let mut name = Buf::<60>::new();
    let _ = write!(name, "PolyRt{me}x{n}");
    let evname = wide(name.as_str());
    // SAFETY: valid name.
    let ev = unsafe { CreateEventW(core::ptr::null(), 1, 0, evname.as_ptr()) };
    let (exe, _) = self_path();
    let mut cmd = [0u16; 600];
    let mut k = 0;
    cmd[k] = b'"' as u16;
    k += 1;
    for &u in wstr_at(exe.as_ptr(), 299) {
        cmd[k] = u;
        k += 1;
    }
    let mut tail = Buf::<160>::new();
    let _ = write!(tail, "\" --child {kind} {}", name.as_str());
    for b in tail.as_str().bytes() {
        cmd[k] = u16::from(b);
        k += 1;
    }
    let mut si = StartupInfoW { cb: 104, pad0: 0, rest: [0; 12] };
    let mut pi = ProcInfo { process: nul(), thread: nul(), pid: 0, tid: 0 };
    // SAFETY: valid buffers.
    let ok = unsafe {
        CreateProcessW(
            exe.as_ptr(),
            cmd.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            0,
            flags,
            core::ptr::null(),
            core::ptr::null(),
            &mut si,
            &mut pi,
        )
    };
    if ok == 0 {
        return Child { process: nul(), thread: nul(), ev, pid: 0, tid: 0 };
    }
    Child { process: pi.process, thread: pi.thread, ev, pid: pi.pid, tid: pi.tid }
}

/// Wait until the target has signalled that it is running its own code.
fn wait_ready(c: &Child) -> bool {
    // SAFETY: valid handle.
    unsafe { WaitForSingleObject(c.ev, 5000) == 0 }
}

/// A ready target of kind "sleep".
fn target() -> Child {
    let c = spawn("sleep", 0);
    if !c.process.is_null() {
        wait_ready(&c);
    }
    c
}

fn kill(c: Child) {
    if !c.process.is_null() {
        // SAFETY: handles from CreateProcessW.
        unsafe {
            TerminateProcess(c.process, 0xDEAD);
            WaitForSingleObject(c.process, 5000);
        }
    }
    close(c.process);
    close(c.thread);
    close(c.ev);
}

fn alive(h: HANDLE) -> bool {
    // SAFETY: valid handle.
    unsafe { WaitForSingleObject(h, 0) == WAIT_TIMEOUT }
}

fn exit_code_of_process(h: HANDLE) -> i64 {
    let mut c = 0u32;
    // SAFETY: valid buffer.
    let ok = unsafe { GetExitCodeProcess(h, &mut c) };
    if ok == 0 { -1 } else { i64::from(c) }
}

fn exit_code_of_thread(h: HANDLE) -> i64 {
    let mut c = 0u32;
    // SAFETY: valid buffer.
    let ok = unsafe { GetExitCodeThread(h, &mut c) };
    if ok == 0 { -1 } else { i64::from(c) }
}

/// Wait up to `ms` for a handle: 0 signalled, 0x102 timeout, 0xFFFFFFFF failed.
fn wait(h: HANDLE, ms: u32) -> u32 {
    // SAFETY: valid handle.
    unsafe { WaitForSingleObject(h, ms) }
}

/// The access mask NtQueryObject reports for a handle (PUBLIC_OBJECT_BASIC_INFORMATION), or 0xFFFFFFFF.
fn granted(h: HANDLE) -> u32 {
    let mut b = ObjBasic { attributes: 0, granted: 0, handle_count: 0, pointer_count: 0, rest: [0; 10] };
    let mut r = 0u32;
    // SAFETY: valid buffer.
    let st = unsafe { NtQueryObject(h, 0, &mut b as *mut ObjBasic as *mut c_void, 56, &mut r) };
    if st < 0 { 0xFFFF_FFFF } else { b.granted }
}

/// A copy of the handle `h` with exactly the rights `rights`, or null.
fn limited(h: HANDLE, rights: u32) -> HANDLE {
    let mut out = nul();
    // SAFETY: valid handles.
    let ok = unsafe { DuplicateHandle(GetCurrentProcess(), h, GetCurrentProcess(), &mut out, rights, 0, 0) };
    if ok == 0 { nul() } else { out }
}

// ---- memory of the target

fn read_u32(p: HANDLE, addr: usize) -> Option<u32> {
    let mut v = [0u8; 4];
    let mut n = 0usize;
    // SAFETY: valid buffer.
    let ok = unsafe { ReadProcessMemory(p, addr, v.as_mut_ptr(), 4, &mut n) };
    if ok == 0 || n != 4 { None } else { Some(u32::from_le_bytes(v)) }
}

fn read_usize(p: HANDLE, addr: usize) -> Option<usize> {
    let mut v = [0u8; 8];
    let mut n = 0usize;
    // SAFETY: valid buffer.
    let ok = unsafe { ReadProcessMemory(p, addr, v.as_mut_ptr(), 8, &mut n) };
    if ok == 0 || n != 8 { None } else { Some(usize::from_le_bytes(v)) }
}

fn write_mem(p: HANDLE, addr: usize, data: &[u8]) -> bool {
    let mut n = 0usize;
    // SAFETY: valid buffer.
    let ok = unsafe { WriteProcessMemory(p, addr, data.as_ptr(), data.len(), &mut n) };
    ok != 0 && n == data.len()
}

/// The machine code of a start routine written into the target: `u32 f(Data *d)`; it stores the id of the
/// process and of the thread it runs in (it calls two kernel32 functions whose addresses are in the data
/// block), its stack pointer and a marker into the block, and returns 0x77.
const CODE: [u8; 41] = [
    0x53, // push rbx
    0x48, 0x83, 0xEC, 0x20, // sub rsp,0x20
    0x48, 0x89, 0xCB, // mov rbx,rcx
    0xFF, 0x13, // call [rbx]
    0x89, 0x43, 0x10, // mov [rbx+0x10],eax
    0xFF, 0x53, 0x08, // call [rbx+8]
    0x89, 0x43, 0x14, // mov [rbx+0x14],eax
    0x48, 0x89, 0x63, 0x20, // mov [rbx+0x20],rsp
    0xC7, 0x43, 0x1C, 0xED, 0x5E, 0xDE, 0xC0, // mov dword [rbx+0x1C],0xC0DE5EED
    0xB8, 0x77, 0x00, 0x00, 0x00, // mov eax,0x77
    0x48, 0x83, 0xC4, 0x20, // add rsp,0x20
    0x5B, // pop rbx
    0xC3, // ret
];
const MARKER: u32 = 0xC0DE_5EED;
const DATA_OFF: usize = 0x100;

/// Code and data in one new page of the target.
struct Inj {
    page: usize,
}

impl Inj {
    fn start(&self) -> usize {
        self.page
    }
    fn param(&self) -> usize {
        self.page + DATA_OFF
    }
    fn marker(&self, p: HANDLE) -> Option<u32> {
        read_u32(p, self.page + DATA_OFF + 0x1C)
    }
    fn pid(&self, p: HANDLE) -> Option<u32> {
        read_u32(p, self.page + DATA_OFF + 0x10)
    }
    fn tid(&self, p: HANDLE) -> Option<u32> {
        read_u32(p, self.page + DATA_OFF + 0x14)
    }
    fn sp(&self, p: HANDLE) -> Option<usize> {
        read_usize(p, self.page + DATA_OFF + 0x20)
    }
}

/// Allocate an executable page in `p` and write the start routine and its data block (the addresses of
/// GetCurrentProcessId and GetCurrentThreadId, which kernel32 has at the same address in every process).
fn inject(p: HANDLE) -> Option<Inj> {
    // SAFETY: plain call.
    let page = unsafe { VirtualAllocEx(p, 0, 0x1000, MEM_COMMIT_RESERVE, 0x40) };
    if page == 0 {
        return None;
    }
    let mut data = [0u8; 0x30];
    data[0..8].copy_from_slice(&k32("GetCurrentProcessId").to_le_bytes());
    data[8..16].copy_from_slice(&k32("GetCurrentThreadId").to_le_bytes());
    if !write_mem(p, page, &CODE) || !write_mem(p, page + DATA_OFF, &data) {
        return None;
    }
    Some(Inj { page })
}

// ---- the calls under test, with the three error reports

struct Made {
    h: HANDLE,
    tid: u32,
    err: i64,
    st: u32,
}

impl Made {
    fn ok(&self) -> bool {
        !self.h.is_null()
    }
    fn text(&self) -> Buf<80> {
        let mut b = Buf::<80>::new();
        if self.ok() {
            let _ = write!(b, "handle");
        } else {
            let _ = write!(b, "NULL gle={} status={:#x}", self.err, self.st);
        }
        b
    }
}

/// kernel32!CreateRemoteThread.
fn crt(p: HANDLE, stack: usize, start: usize, arg: usize, flags: u32) -> Made {
    let mut tid = 0xAAAA_AAAAu32;
    // SAFETY: plain call.
    unsafe {
        SetLastError(0xDEAD);
        let h = CreateRemoteThread(p, core::ptr::null(), stack, start, arg, flags, &mut tid);
        let err = gle();
        let st = status_now();
        Made { h, tid, err, st }
    }
}

/// kernel32!CreateRemoteThreadEx.
fn crte(p: HANDLE, stack: usize, start: usize, arg: usize, flags: u32, attrs: *mut c_void) -> Made {
    let mut tid = 0xAAAA_AAAAu32;
    // SAFETY: plain call.
    unsafe {
        SetLastError(0xDEAD);
        let h = CreateRemoteThreadEx(p, core::ptr::null(), stack, start, arg, flags, attrs, &mut tid);
        let err = gle();
        let st = status_now();
        Made { h, tid, err, st }
    }
}

/// ntdll!NtCreateThreadEx: (status, handle).
fn nct(
    p: HANDLE,
    access: u32,
    start: usize,
    arg: usize,
    flags: u32,
    zero: usize,
    stack: usize,
    maxstack: usize,
) -> (i32, HANDLE) {
    let mut h = nul();
    // SAFETY: plain call.
    let st = unsafe {
        NtCreateThreadEx(
            &mut h,
            access,
            core::ptr::null(),
            p,
            start,
            arg,
            flags,
            zero,
            stack,
            maxstack,
            core::ptr::null_mut(),
        )
    };
    (st, h)
}

fn yn(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

fn opt_hex(v: Option<u32>) -> Buf<24> {
    let mut b = Buf::<24>::new();
    match v {
        Some(x) => {
            let _ = write!(b, "{x:#x}");
        }
        None => {
            let _ = write!(b, "unreadable");
        }
    }
    b
}

/// Let a new thread run to its end: resume it (a no-op for a running one), wait for it, and return the wait
/// result and the exit code.
fn run_out(h: HANDLE) -> (u32, i64) {
    // SAFETY: valid handle.
    unsafe { ResumeThread(h) };
    let w = wait(h, 10000);
    (w, exit_code_of_thread(h))
}

/// The stack of the live thread `h` of the process `p`: (reserved bytes, committed bytes, guard pages).
fn stack_of(p: HANDLE, h: HANDLE) -> Option<(usize, usize, usize)> {
    let mut tb =
        ThreadBasic { exit_status: 0, pad: 0, teb: 0, pid: 0, tid: 0, affinity: 0, priority: 0, base_priority: 0 };
    let mut r = 0u32;
    // SAFETY: valid buffer.
    let st = unsafe { NtQueryInformationThread(h, 0, &mut tb as *mut ThreadBasic as *mut c_void, 48, &mut r) };
    if st < 0 {
        return None;
    }
    let base = read_usize(p, tb.teb + 8)?;
    let mut mi = MemInfo {
        base: 0,
        alloc_base: 0,
        alloc_protect: 0,
        partition: 0,
        pad: 0,
        size: 0,
        state: 0,
        protect: 0,
        kind: 0,
        pad2: 0,
    };
    // SAFETY: valid buffer.
    let n = unsafe { VirtualQueryEx(p, base - 1, &mut mi, 48) };
    if n == 0 {
        return None;
    }
    let alloc = mi.alloc_base;
    let (mut commit, mut guards) = (0usize, 0usize);
    let mut a = alloc;
    while a < base {
        // SAFETY: valid buffer.
        let n = unsafe { VirtualQueryEx(p, a, &mut mi, 48) };
        if n == 0 || mi.size == 0 {
            return None;
        }
        if mi.state == 0x1000 {
            commit += mi.size;
        }
        if mi.protect & 0x100 != 0 {
            guards += mi.size / 0x1000;
        }
        a += mi.size;
    }
    Some((base - alloc, commit, guards))
}

/// The SizeOfStackReserve and SizeOfStackCommit fields of this image's own PE optional header.
fn image_stack() -> (usize, usize) {
    // SAFETY: the image of this process is mapped at its module handle.
    unsafe {
        let base = GetModuleHandleW(core::ptr::null()) as usize;
        let pe = *((base + 0x3c) as *const u32) as usize;
        let opt = base + pe + 24;
        (*((opt + 72) as *const u64) as usize, *((opt + 80) as *const u64) as usize)
    }
}

/// One line about the stack of a remote thread created with `stack` and `flags`.
fn stack_line(t: &mut Test, label: &str, c: &Child, stack: usize, flags: u32) {
    let sleep = k32("Sleep");
    let m = crt(c.process, stack, sleep, 60000, flags);
    if !m.ok() {
        say!(t, "{label}: create {}", m.text().as_str());
        return;
    }
    // SAFETY: valid handle.
    unsafe { ResumeThread(m.h) };
    let mut r = stack_of(c.process, m.h);
    let mut tries = 0;
    while (r.is_none() || r.map(|x| x.1) == Some(0)) && tries < 200 {
        // SAFETY: plain call.
        unsafe { Sleep(25) };
        r = stack_of(c.process, m.h);
        tries += 1;
    }
    match r {
        Some((res, com, _g)) => {
            if flags & STACK_SIZE_PARAM_IS_A_RESERVATION == 0 && stack != 0 {
                say!(t, "{label}: reserve={}K, committed at least the requested size: {}", res / 1024, yn(com >= stack))
            } else {
                say!(t, "{label}: reserve={}K, committed at least one page: {}", res / 1024, yn(com >= 0x1000))
            }
        }
        None => say!(t, "{label}: stack not readable"),
    }
    close(m.h);
}

// ---------------------------------------------------------------- the child role

fn child_main(args: &str) -> u32 {
    // SAFETY: plain calls.
    unsafe { SetErrorMode(0x8003) };
    let mut it = args.split(' ');
    let kind = it.next().unwrap_or("");
    let name = it.next().unwrap_or("");
    // `h<hex>`: an inherited handle to the event; otherwise the name of an event
    let ev = if let Some(hex) = name.strip_prefix('h') {
        usize::from_str_radix(hex, 16).unwrap_or(0) as HANDLE
    } else {
        let w = wide(name);
        // SAFETY: valid name.
        unsafe { OpenEventW(2, 0, w.as_ptr()) }
    };
    if !ev.is_null() {
        // SAFETY: valid handle.
        unsafe { SetEvent(ev) };
    }
    match kind {
        "exit" => 7,
        _ => loop {
            // SAFETY: plain call.
            unsafe { Sleep(INFINITE) };
        },
    }
}

macro_rules! main_rt {
    ($test:path) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn mainCRTStartup() -> ! {
            if let Some(a) = child_args() {
                exit(child_main(a.as_str()))
            }
            if open_output().is_err() {
                exit(EXIT_NO_OUTPUT);
            }
            let mut t = Test::new();
            $test(&mut t);
            let code = t.finish();
            exit(code)
        }
    };
}

// ---------------------------------------------------------------- thread ideal processor and group affinity helpers

#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessorNumber {
    group: u16,
    number: u8,
    reserved: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct GroupAffinity {
    mask: usize,
    group: u16,
    reserved: [u16; 3],
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetThreadIdealProcessorEx(h: HANDLE, pn: *mut ProcessorNumber) -> BOOL;
    fn SetThreadIdealProcessorEx(h: HANDLE, pn: *const ProcessorNumber, prev: *mut ProcessorNumber) -> BOOL;
    fn GetThreadGroupAffinity(h: HANDLE, ga: *mut GroupAffinity) -> BOOL;
    fn SetThreadGroupAffinity(h: HANDLE, ga: *const GroupAffinity, prev: *mut GroupAffinity) -> BOOL;
    fn GetProcessAffinityMask(h: HANDLE, p: *mut usize, s: *mut usize) -> BOOL;
    fn GetActiveProcessorCount(group: u16) -> u32;
    fn TerminateThread(h: HANDLE, c: u32) -> BOOL;
}

/// The number of active processors of group 0.
fn cpus() -> u32 {
    // SAFETY: plain call.
    unsafe { GetActiveProcessorCount(0) }
}

/// The affinity mask of a process.
fn proc_mask(p: HANDLE) -> usize {
    let (mut pm, mut sm) = (0usize, 0usize);
    // SAFETY: valid buffers.
    unsafe { GetProcessAffinityMask(p, &mut pm, &mut sm) };
    pm
}

fn ideal(h: HANDLE) -> Result<ProcessorNumber, i64> {
    let mut pn = ProcessorNumber { group: 77, number: 99, reserved: 55 };
    // SAFETY: valid buffer.
    let ok = unsafe {
        SetLastError(0xDEAD);
        GetThreadIdealProcessorEx(h, &mut pn)
    };
    if ok == 0 { Err(gle()) } else { Ok(pn) }
}

fn affinity(h: HANDLE) -> Result<GroupAffinity, i64> {
    let mut g = GroupAffinity { mask: 0x55, group: 77, reserved: [9; 3] };
    // SAFETY: valid buffer.
    let ok = unsafe {
        SetLastError(0xDEAD);
        GetThreadGroupAffinity(h, &mut g)
    };
    if ok == 0 { Err(gle()) } else { Ok(g) }
}

/// What GetThreadIdealProcessorEx says about `h`, as text without machine-specific numbers.
fn ideal_text(h: HANDLE) -> Buf<120> {
    let mut b = Buf::<120>::new();
    match ideal(h) {
        Ok(p) => {
            let _ = write!(
                b,
                "ok: group {}, number below the processor count {}, reserved {}",
                p.group,
                yn(u32::from(p.number) < cpus()),
                p.reserved
            );
        }
        Err(e) => {
            let _ = write!(b, "error {e}");
        }
    }
    b
}

/// What GetThreadGroupAffinity says about `h`; `pm` is the affinity mask of the thread's process.
fn affinity_text(h: HANDLE, pm: usize) -> Buf<160> {
    let mut b = Buf::<160>::new();
    match affinity(h) {
        Ok(g) => {
            let _ = write!(
                b,
                "ok: group {}, mask not zero {}, mask inside the process mask {}, mask equals the process mask {}, reserved all zero {}",
                g.group,
                yn(g.mask != 0),
                yn(g.mask & !pm == 0),
                yn(g.mask == pm),
                yn(g.reserved == [0; 3])
            );
        }
        Err(e) => {
            let _ = write!(b, "error {e}");
        }
    }
    b
}

/// A new thread of this process, suspended, that is never resumed (ended by `drop_thread`).
fn new_thread() -> HANDLE {
    // SAFETY: plain call.
    unsafe { CreateThread(core::ptr::null(), 0, k32("Sleep"), 1000, CREATE_SUSPENDED, core::ptr::null_mut()) }
}

fn drop_thread(h: HANDLE) {
    // SAFETY: a suspended thread this test created.
    unsafe { TerminateThread(h, 0) };
    close(h);
}

/// A real handle (all rights) to the calling thread.
fn own_thread() -> HANDLE {
    // SAFETY: plain calls.
    unsafe { OpenThread(THREAD_ALL_ACCESS, 0, GetCurrentThreadId()) }
}

fn set_ideal(h: HANDLE, group: u16, number: u8, reserved: u8) -> (bool, i64, Option<ProcessorNumber>) {
    let want = ProcessorNumber { group, number, reserved };
    let mut prev = ProcessorNumber { group: 77, number: 99, reserved: 55 };
    // SAFETY: valid buffers.
    unsafe {
        SetLastError(0xDEAD);
        let ok = SetThreadIdealProcessorEx(h, &want, &mut prev);
        (ok != 0, if ok == 0 { gle() } else { -2 }, if ok != 0 { Some(prev) } else { None })
    }
}

fn set_affinity(h: HANDLE, mask: usize, group: u16, reserved: u16) -> (bool, i64, Option<GroupAffinity>) {
    let want = GroupAffinity { mask, group, reserved: [reserved, 0, 0] };
    let mut prev = GroupAffinity { mask: 0x55, group: 77, reserved: [9; 3] };
    // SAFETY: valid buffers.
    unsafe {
        SetLastError(0xDEAD);
        let ok = SetThreadGroupAffinity(h, &want, &mut prev);
        (ok != 0, if ok == 0 { gle() } else { -2 }, if ok != 0 { Some(prev) } else { None })
    }
}

fn rt(r: &(bool, i64)) -> Buf<30> {
    let mut b = Buf::<30>::new();
    if r.0 {
        let _ = write!(b, "ok");
    } else {
        let _ = write!(b, "error {}", r.1);
    }
    b
}
main_rt!(test);

fn test(t: &mut Test) {
    // SAFETY: plain call.
    let pm = proc_mask(unsafe { GetCurrentProcess() });
    let real = own_thread();
    for (name, rights) in [
        ("no rights", 0u32),
        ("THREAD_TERMINATE", 1),
        ("THREAD_SUSPEND_RESUME", 2),
        ("THREAD_SET_INFORMATION", 0x20),
        ("THREAD_QUERY_INFORMATION", 0x40),
        ("THREAD_SET_LIMITED_INFORMATION", 0x400),
        ("THREAD_QUERY_LIMITED_INFORMATION", 0x800),
        ("THREAD_RESUME", 0x1000),
        ("SYNCHRONIZE", 0x100000),
        ("all rights", 0x1FFFFF),
    ] {
        let d = limited(real, rights);
        say!(t, "{name}: {}", affinity_text(d, pm).as_str());
        close(d);
    }
    close(real);
}
