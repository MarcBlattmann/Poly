// This program is imp_threads, recorded with the standard (filtered) token.
//! ImpersonateNamedPipeClient is per thread: a second thread of the server has no token while the
//! first impersonates; the thread that reads need not be the one that impersonates; a thread may
//! impersonate several clients one after the other on one pipe instance (disconnect, connect the
//! next client, read, impersonate); RevertToSelf and SetThreadToken(NULL) both end it.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, static_mut_refs, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the public SDK headers
// winbase.h, winnt.h, securitybaseapi.h and winerror.h (constants, layouts); the helpers are this
// test's own.

// ---- shared helpers of the ImpersonateNamedPipeClient tests (this kit's own code) ----
use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
type PSID = *mut c_void;

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
        let mut b = Buf::<400>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// winnt.h SID_AND_ATTRIBUTES
#[repr(C)]
#[derive(Clone, Copy)]
struct SID_AND_ATTRIBUTES {
    sid: PSID,
    attributes: u32,
}

#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    descriptor: *mut c_void,
    inherit: BOOL,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentThread() -> HANDLE;
    fn GetCurrentProcess() -> HANDLE;
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        max_instances: u32,
        out_buf: u32,
        in_buf: u32,
        timeout: u32,
        sa: *const SECURITY_ATTRIBUTES,
    ) -> HANDLE;
    fn ConnectNamedPipe(h: HANDLE, overlapped: *mut c_void) -> BOOL;
    fn DisconnectNamedPipe(h: HANDLE) -> BOOL;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *const SECURITY_ATTRIBUTES,
        disposition: u32,
        flags: u32,
        template: HANDLE,
    ) -> HANDLE;
    fn ReadFile(h: HANDLE, buf: *mut u8, n: u32, read: *mut u32, overlapped: *mut c_void) -> BOOL;
    fn WriteFile(h: HANDLE, buf: *const u8, n: u32, written: *mut u32, overlapped: *mut c_void) -> BOOL;
    fn CreatePipe(read: *mut HANDLE, write: *mut HANDLE, sa: *const SECURITY_ATTRIBUTES, size: u32) -> BOOL;
    fn CreateEventW(sa: *const SECURITY_ATTRIBUTES, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn SetEvent(h: HANDLE) -> BOOL;
    fn Sleep(ms: u32);
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn CreateThread(
        attrs: *mut c_void,
        stack: usize,
        start: unsafe extern "system" fn(*mut c_void) -> u32,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ImpersonateNamedPipeClient(pipe: HANDLE) -> BOOL;
    fn RevertToSelf() -> BOOL;
    fn ImpersonateSelf(level: i32) -> BOOL;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    fn OpenThreadToken(thread: HANDLE, access: u32, as_self: BOOL, token: *mut HANDLE) -> BOOL;
    fn SetThreadToken(thread: *const HANDLE, token: HANDLE) -> BOOL;
    fn GetTokenInformation(token: HANDLE, class: i32, buf: *mut c_void, len: u32, ret_len: *mut u32) -> BOOL;
    fn SetTokenInformation(token: HANDLE, class: i32, buf: *const c_void, len: u32) -> BOOL;
    fn DuplicateTokenEx(token: HANDLE, access: u32, sa: *const c_void, level: i32, ty: i32, out: *mut HANDLE) -> BOOL;
    fn CreateRestrictedToken(
        token: HANDLE,
        flags: u32,
        n_disable: u32,
        sids_disable: *const SID_AND_ATTRIBUTES,
        n_delete: u32,
        privs_delete: *const c_void,
        n_restrict: u32,
        sids_restrict: *const SID_AND_ATTRIBUTES,
        out: *mut HANDLE,
    ) -> BOOL;
    fn EqualSid(a: PSID, b: PSID) -> BOOL;
    fn CheckTokenMembership(token: HANDLE, sid: PSID, is_member: *mut BOOL) -> BOOL;
    fn AllocateAndInitializeSid(
        authority: *const [u8; 6],
        count: u8,
        s0: u32,
        s1: u32,
        s2: u32,
        s3: u32,
        s4: u32,
        s5: u32,
        s6: u32,
        s7: u32,
        out: *mut PSID,
    ) -> BOOL;
    fn FreeSid(sid: PSID) -> *mut c_void;
    fn ConvertSidToStringSidW(sid: PSID, out: *mut *mut u16) -> BOOL;
    fn GetLengthSid(sid: PSID) -> u32;
    fn IsTokenRestricted(token: HANDLE) -> BOOL;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl: *const u16,
        revision: u32,
        sd: *mut *mut c_void,
        size: *mut u32,
    ) -> BOOL;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LocalFree(p: *mut c_void) -> *mut c_void;
}

const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_DUPLICATE: u32 = 0x0002;
const TOKEN_IMPERSONATE: u32 = 0x0004;
const TOKEN_ADJUST_DEFAULT: u32 = 0x0080;
const TOKEN_ALL_ACCESS_ISH: u32 = 0xF01FF;
const GENERIC_READ_WRITE: u32 = 0xC000_0000;
const OPEN_EXISTING: u32 = 3;
const PIPE_ACCESS_DUPLEX: u32 = 3;
const SECURITY_SQOS_PRESENT: u32 = 0x0010_0000;
const SECURITY_ANONYMOUS: u32 = 0;
const SECURITY_IDENTIFICATION: u32 = 1 << 16;
const SECURITY_IMPERSONATION: u32 = 2 << 16;
const SECURITY_DELEGATION: u32 = 3 << 16;
const SECURITY_CONTEXT_TRACKING: u32 = 0x0004_0000;
const SECURITY_EFFECTIVE_ONLY: u32 = 0x0008_0000;
const INVALID_HANDLE: HANDLE = -1isize as HANDLE;

// TOKEN_INFORMATION_CLASS (winnt.h)
const TOKEN_USER: i32 = 1;
const TOKEN_GROUPS: i32 = 2;
const TOKEN_PRIVILEGES: i32 = 3;
const TOKEN_TYPE: i32 = 8;
const TOKEN_IMPERSONATION_LEVEL: i32 = 9;
const TOKEN_SESSION_ID: i32 = 12;
const TOKEN_ELEVATION_TYPE: i32 = 18;
const TOKEN_ELEVATION: i32 = 20;
const TOKEN_INTEGRITY_LEVEL: i32 = 25;

fn wide(s: &str) -> [u16; 80] {
    let mut o = [0u16; 80];
    for (i, c) in s.bytes().enumerate() {
        o[i] = c as u16;
    }
    o
}

/// The pipe name `\\.\pipe\PolySpecImp<tag>`.
fn pipe_name(tag: &str) -> [u16; 80] {
    let mut s = Buf::<80>::new();
    let _ = write!(s, "\\\\.\\pipe\\PolySpecImp{}", tag);
    wide(s.as_str())
}

/// The server end of a one-instance duplex byte pipe whose security descriptor lets everybody
/// connect, also clients with a low integrity level or a restricted token:
/// D:(A;;GA;;;WD)S:(ML;;NW;;;LW) (the updater's pipe has a descriptor of its own too).
fn make_pipe(tag: &str) -> HANDLE {
    let name = pipe_name(tag);
    let sddl = wide("D:(A;;GA;;;WD)S:(ML;;NW;;;LW)");
    let mut sd: *mut c_void = core::ptr::null_mut();
    // SAFETY: NUL terminated strings that outlive the calls; the descriptor is kept for the run.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut sd, core::ptr::null_mut());
        let sa = SECURITY_ATTRIBUTES {
            length: core::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            descriptor: sd,
            inherit: 0,
        };
        CreateNamedPipeW(name.as_ptr(), PIPE_ACCESS_DUPLEX, 0, 1, 512, 512, 0, &sa)
    }
}

/// What a client thread does.
struct ClientJob {
    tag: [u8; 24],
    /// Flags for CreateFileW (SECURITY_SQOS_PRESENT | level ...); 0: none given.
    flags: u32,
    /// Write one byte after connecting.
    write: bool,
    /// 0: use the thread's own (process) identity; 1: impersonate a low integrity copy of the
    /// process token; 2: impersonate a restricted copy with Administrators deny-only and no
    /// privileges but the one kept (DISABLE_MAX_PRIVILEGE).
    identity: u32,
    up: HANDLE,
    go: HANDLE,
    result_err: u32,
    result_open: bool,
}

unsafe extern "system" fn client_main(p: *mut c_void) -> u32 {
    // SAFETY: the job outlives the thread (the test waits for it).
    let job = unsafe { &mut *(p as *mut ClientJob) };
    let tag = core::str::from_utf8(&job.tag).unwrap_or("").trim_end_matches('\0');
    let name = pipe_name(tag);
    // SAFETY: plain calls with live buffers and handles of this test.
    unsafe {
        if job.identity != 0 {
            let mut tok: HANDLE = core::ptr::null_mut();
            OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS_ISH, &mut tok);
            let mut imp: HANDLE = core::ptr::null_mut();
            if job.identity == 1 {
                DuplicateTokenEx(tok, TOKEN_ALL_ACCESS_ISH, core::ptr::null(), 2, 2, &mut imp);
                let mut sid: PSID = core::ptr::null_mut();
                let auth = [0u8, 0, 0, 0, 0, 16];
                AllocateAndInitializeSid(&auth, 1, 4096, 0, 0, 0, 0, 0, 0, 0, &mut sid);
                let label = SID_AND_ATTRIBUTES { sid, attributes: 0x20 };
                SetTokenInformation(
                    imp,
                    TOKEN_INTEGRITY_LEVEL,
                    &label as *const _ as *const c_void,
                    8 + GetLengthSid(sid),
                );
            } else {
                let mut admins: PSID = core::ptr::null_mut();
                let auth = [0u8, 0, 0, 0, 0, 5];
                AllocateAndInitializeSid(&auth, 2, 32, 544, 0, 0, 0, 0, 0, 0, &mut admins);
                let dis = SID_AND_ATTRIBUTES { sid: admins, attributes: 0 };
                let mut restricted: HANDLE = core::ptr::null_mut();
                CreateRestrictedToken(tok, 1, 1, &dis, 0, core::ptr::null(), 0, core::ptr::null(), &mut restricted);
                DuplicateTokenEx(restricted, TOKEN_ALL_ACCESS_ISH, core::ptr::null(), 2, 2, &mut imp);
            }
            SetThreadToken(core::ptr::null(), imp);
        }
        // The server may not be listening yet (after DisconnectNamedPipe the instance is busy until the next
        // ConnectNamedPipe): try again while the answer is ERROR_PIPE_BUSY (231), as callers of WaitNamedPipe do.
        // Without it the server's ConnectNamedPipe waited for ever when this thread ran first (a CI timeout).
        let mut h;
        let mut tries = 0;
        loop {
            h = CreateFileW(
                name.as_ptr(),
                GENERIC_READ_WRITE,
                0,
                core::ptr::null(),
                OPEN_EXISTING,
                job.flags,
                core::ptr::null_mut(),
            );
            if h != INVALID_HANDLE || GetLastError() != 231 || tries >= 4000 {
                break;
            }
            tries += 1;
            Sleep(5);
        }
        job.result_open = h != INVALID_HANDLE;
        job.result_err = GetLastError();
        if job.identity != 0 {
            RevertToSelf();
        }
        if h != INVALID_HANDLE && job.write {
            let mut n = 0u32;
            WriteFile(h, b"x".as_ptr(), 1, &mut n, core::ptr::null_mut());
        }
        SetEvent(job.up);
        WaitForSingleObject(job.go, 20000);
        if h != INVALID_HANDLE {
            CloseHandle(h);
        }
    }
    0
}

/// A started client: wait for it with `up()`, let it end with `finish()`.
struct Client {
    job: ClientJob,
    thread: HANDLE,
}

impl Client {
    fn start(tag: &str, flags: u32, write: bool, identity: u32) -> Box2 {
        let mut t = [0u8; 24];
        t[..tag.len()].copy_from_slice(tag.as_bytes());
        // SAFETY: events and a thread of this test; the job lives in a static slot.
        unsafe {
            let slot = next_slot();
            *slot = Some(Client {
                job: ClientJob {
                    tag: t,
                    flags,
                    write,
                    identity,
                    up: CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()),
                    go: CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()),
                    result_err: 0,
                    result_open: false,
                },
                thread: core::ptr::null_mut(),
            });
            let c = slot.as_mut().unwrap();
            let mut id = 0u32;
            c.thread = CreateThread(
                core::ptr::null_mut(),
                0,
                client_main,
                &mut c.job as *mut ClientJob as *mut c_void,
                0,
                &mut id,
            );
            Box2(c as *mut Client)
        }
    }
}

/// A pointer to a client in a static slot (no allocator in these tests).
struct Box2(*mut Client);

impl Box2 {
    /// Waits until the client has connected (and written, if it writes); returns whether it
    /// could open the pipe and the error CreateFileW left.
    fn up(&self) -> (bool, u32) {
        // SAFETY: the slot lives for the whole run.
        unsafe {
            let c = &*self.0;
            WaitForSingleObject(c.job.up, 20000);
            (c.job.result_open, c.job.result_err)
        }
    }
    /// Lets the client close its end and waits for its thread.
    fn finish(&self) {
        // SAFETY: the slot lives for the whole run.
        unsafe {
            let c = &*self.0;
            SetEvent(c.job.go);
            WaitForSingleObject(c.thread, 20000);
        }
    }
}

static mut SLOTS: [Option<Client>; 24] = [const { None }; 24];
static mut NEXT: usize = 0;

#[allow(static_mut_refs)]
unsafe fn next_slot() -> &'static mut Option<Client> {
    // SAFETY: only the main thread starts clients.
    unsafe {
        let i = NEXT % 24;
        NEXT += 1;
        &mut SLOTS[i]
    }
}

/// Reads the one byte the client wrote.
fn read_byte(pipe: HANDLE) -> (bool, u32) {
    let mut b = [0u8; 4];
    let mut n = 0u32;
    // SAFETY: a plain call with live buffers.
    unsafe {
        SetLastError(0);
        let r = ReadFile(pipe, b.as_mut_ptr(), 1, &mut n, core::ptr::null_mut());
        (r != 0 && n == 1, GetLastError())
    }
}

fn open_thread_token(as_self: bool) -> Option<HANDLE> {
    let mut t: HANDLE = core::ptr::null_mut();
    // SAFETY: a live out handle.
    let ok = unsafe {
        OpenThreadToken(GetCurrentThread(), TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_IMPERSONATE, as_self as BOOL, &mut t)
    };
    if ok != 0 { Some(t) } else { None }
}

static mut PTOK: HANDLE = core::ptr::null_mut();

/// The process token, opened once (the first call must come before any impersonation: a thread
/// impersonating at the identification level cannot open it).
fn process_token() -> HANDLE {
    // SAFETY: a live out handle; single threaded use.
    unsafe {
        if PTOK.is_null() {
            let mut t: HANDLE = core::ptr::null_mut();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY | TOKEN_DUPLICATE, &mut t);
            PTOK = t;
        }
        PTOK
    }
}

fn info_u32(tok: HANDLE, class: i32) -> Option<u32> {
    let mut v = 0u32;
    let mut n = 0u32;
    // SAFETY: a 4 byte out buffer.
    let ok = unsafe { GetTokenInformation(tok, class, &mut v as *mut u32 as *mut c_void, 4, &mut n) };
    if ok != 0 { Some(v) } else { None }
}

#[repr(C, align(8))]
struct Big([u8; 4096]);

fn groups_same(a: HANDLE, b: HANDLE) -> (bool, u32) {
    let (mut ba, mut bb) = (Big([0; 4096]), Big([0; 4096]));
    let (mut na, mut nb) = (0u32, 0u32);
    // SAFETY: 4096 byte buffers, SIDs are read from them while they are alive.
    unsafe {
        let ra = GetTokenInformation(a, TOKEN_GROUPS, ba.0.as_mut_ptr() as *mut c_void, 4096, &mut na);
        let rb = GetTokenInformation(b, TOKEN_GROUPS, bb.0.as_mut_ptr() as *mut c_void, 4096, &mut nb);
        if ra == 0 || rb == 0 {
            return (false, 0);
        }
        let ca = *(ba.0.as_ptr() as *const u32);
        let cb = *(bb.0.as_ptr() as *const u32);
        let arr_a = (ba.0.as_ptr() as *const u8).add(core::mem::align_of::<SID_AND_ATTRIBUTES>().max(4))
            as *const SID_AND_ATTRIBUTES;
        let arr_b = (bb.0.as_ptr() as *const u8).add(core::mem::align_of::<SID_AND_ATTRIBUTES>().max(4))
            as *const SID_AND_ATTRIBUTES;
        let mut deny_only = 0;
        let mut same = ca == cb;
        for i in 0..ca as usize {
            let ea = *arr_a.add(i);
            if ea.attributes & 0x10 != 0 {
                deny_only += 1;
            }
            let mut found = false;
            for j in 0..cb as usize {
                let eb = *arr_b.add(j);
                if EqualSid(ea.sid, eb.sid) != 0 && ea.attributes == eb.attributes {
                    found = true;
                    break;
                }
            }
            if !found {
                same = false;
            }
        }
        (same, deny_only)
    }
}

fn user_same(a: HANDLE, b: HANDLE) -> bool {
    let (mut ba, mut bb) = (Big([0; 4096]), Big([0; 4096]));
    let (mut na, mut nb) = (0u32, 0u32);
    // SAFETY: 4096 byte buffers.
    unsafe {
        let ra = GetTokenInformation(a, TOKEN_USER, ba.0.as_mut_ptr() as *mut c_void, 4096, &mut na);
        let rb = GetTokenInformation(b, TOKEN_USER, bb.0.as_mut_ptr() as *mut c_void, 4096, &mut nb);
        ra != 0
            && rb != 0
            && EqualSid(
                (*(ba.0.as_ptr() as *const SID_AND_ATTRIBUTES)).sid,
                (*(bb.0.as_ptr() as *const SID_AND_ATTRIBUTES)).sid,
            ) != 0
    }
}

fn user_sid_text(a: HANDLE) -> Buf<80> {
    let mut out = Buf::<80>::new();
    let mut ba = Big([0; 4096]);
    let mut na = 0u32;
    // SAFETY: a 4096 byte buffer; the string is freed after copying.
    unsafe {
        if GetTokenInformation(a, TOKEN_USER, ba.0.as_mut_ptr() as *mut c_void, 4096, &mut na) != 0 {
            let mut s: *mut u16 = core::ptr::null_mut();
            if ConvertSidToStringSidW((*(ba.0.as_ptr() as *const SID_AND_ATTRIBUTES)).sid, &mut s) != 0 {
                let mut i = 0;
                while *s.add(i) != 0 && i < 79 {
                    let _ = write!(out, "{}", (*s.add(i) as u8) as char);
                    i += 1;
                }
                LocalFree(s as *mut c_void);
            }
        }
    }
    out
}

fn integrity_rid(tok: HANDLE) -> Option<u32> {
    let mut b = Big([0; 4096]);
    let mut n = 0u32;
    // SAFETY: a 4096 byte buffer; the SID is read from it.
    unsafe {
        if GetTokenInformation(tok, TOKEN_INTEGRITY_LEVEL, b.0.as_mut_ptr() as *mut c_void, 4096, &mut n) == 0 {
            return None;
        }
        let sid = (*(b.0.as_ptr() as *const SID_AND_ATTRIBUTES)).sid as *const u8;
        let subs = *sid.add(1) as usize;
        // SID layout: revision, sub authority count, 6 bytes authority, then the sub authorities
        Some(*(sid.add(8 + 4 * (subs - 1)) as *const u32))
    }
}

fn privilege_count(tok: HANDLE) -> Option<u32> {
    let mut b = Big([0; 4096]);
    let mut n = 0u32;
    // SAFETY: a 4096 byte buffer.
    unsafe {
        if GetTokenInformation(tok, TOKEN_PRIVILEGES, b.0.as_mut_ptr() as *mut c_void, 4096, &mut n) == 0 {
            return None;
        }
        Some(*(b.0.as_ptr() as *const u32))
    }
}

fn is_admin_member(tok: HANDLE) -> i32 {
    let mut admins: PSID = core::ptr::null_mut();
    let auth = [0u8, 0, 0, 0, 0, 5];
    let mut m: BOOL = -1;
    // SAFETY: a SID made and freed here.
    unsafe {
        AllocateAndInitializeSid(&auth, 2, 32, 544, 0, 0, 0, 0, 0, 0, &mut admins);
        let r = CheckTokenMembership(tok, admins, &mut m);
        FreeSid(admins);
        if r == 0 { -1 } else { m }
    }
}

/// One line about the thread token of the calling thread (as opened with OpenAsSelf TRUE).
fn thread_token_line(t: &mut Test, label: &str) {
    let Some(tt) = open_thread_token(true) else {
        // SAFETY: a plain call.
        say!(t, "{}: no thread token (OpenThreadToken error {})", label, unsafe { GetLastError() });
        return;
    };
    let pt = process_token();
    let (gs, deny) = groups_same(tt, pt);
    let mut line = Buf::<400>::new();
    let _ = write!(
        line,
        "{}: type {:?} level {:?} user same as process {} groups same as process {} deny-only groups {}",
        label,
        info_u32(tt, TOKEN_TYPE),
        info_u32(tt, TOKEN_IMPERSONATION_LEVEL),
        user_same(tt, pt),
        gs,
        deny
    );
    t.line(line.as_str());
    let mut line = Buf::<400>::new();
    let mut ir = Buf::<20>::new();
    match integrity_rid(tt) {
        Some(r) => {
            let _ = write!(ir, "{:#x}", r);
        }
        None => {
            let _ = write!(ir, "none");
        }
    }
    let _ = write!(
        line,
        "  integrity {} elevation type {:?} elevated {:?} Administrators member {} restricted {} privileges {:?} session same {}",
        ir.as_str(),
        info_u32(tt, TOKEN_ELEVATION_TYPE),
        info_u32(tt, TOKEN_ELEVATION),
        is_admin_member(core::ptr::null_mut()),
        // SAFETY: a plain call.
        unsafe { IsTokenRestricted(tt) },
        privilege_count(tt),
        info_u32(tt, TOKEN_SESSION_ID) == info_u32(pt, TOKEN_SESSION_ID)
    );
    t.line(line.as_str());
    // SAFETY: the thread token handle of this function.
    unsafe {
        CloseHandle(tt);
    }
}
// ---- end of the shared helpers ----

poly_wtest::main!(test);

static mut OTHER_PIPE: HANDLE = core::ptr::null_mut();
static mut OTHER_HAS_TOKEN: i32 = -1;
static mut OTHER_IMP: i32 = -1;
static mut OTHER_ERR: u32 = 0;
static mut OTHER_TYPE: u32 = 0;
static mut OTHER_LEVEL: u32 = 0;
static mut OTHER_READ: i32 = -1;
static mut DO_READ: bool = false;

/// A second thread: reports whether it has a token, then (if asked) impersonates the pipe's client.
unsafe extern "system" fn other_thread(_: *mut c_void) -> u32 {
    // SAFETY: statics written here are read after the thread was waited for.
    unsafe {
        OTHER_HAS_TOKEN = open_thread_token(true).is_some() as i32;
        if DO_READ {
            OTHER_READ = read_byte(OTHER_PIPE).0 as i32;
        }
        SetLastError(0x5A5A);
        OTHER_IMP = ImpersonateNamedPipeClient(OTHER_PIPE);
        OTHER_ERR = GetLastError();
        if let Some(tt) = open_thread_token(true) {
            OTHER_TYPE = info_u32(tt, TOKEN_TYPE).unwrap_or(99);
            OTHER_LEVEL = info_u32(tt, TOKEN_IMPERSONATION_LEVEL).unwrap_or(99);
            CloseHandle(tt);
        } else {
            OTHER_TYPE = 0;
        }
        RevertToSelf();
    }
    0
}

fn run_other(read: bool) {
    // SAFETY: a thread of this test, waited for.
    unsafe {
        DO_READ = read;
        let mut id = 0u32;
        let th = CreateThread(core::ptr::null_mut(), 0, other_thread, core::ptr::null_mut(), 0, &mut id);
        WaitForSingleObject(th, 20000);
        CloseHandle(th);
    }
}

fn test(t: &mut Test) {
    // opened before any impersonation
    let _ = process_token();
    // SAFETY: calls into kernel32 and advapi32 with handles of this test.
    unsafe {
        let pipe = make_pipe("Th1");
        OTHER_PIPE = pipe;
        let c = Client::start("Th1", 0, true, 0);
        ConnectNamedPipe(pipe, core::ptr::null_mut());
        c.up();
        read_byte(pipe);
        let r = ImpersonateNamedPipeClient(pipe);
        say!(t, "main thread impersonates: {}", r);
        run_other(false);
        say!(
            t,
            "second thread while the main thread impersonates: had a token {} then impersonated {} (error {}) type {} level {}",
            OTHER_HAS_TOKEN,
            OTHER_IMP,
            OTHER_ERR,
            OTHER_TYPE,
            OTHER_LEVEL
        );
        thread_token_line(t, "main thread after the second thread ran");

        // SetThreadToken(NULL, NULL) removes the impersonation as RevertToSelf does
        let r = SetThreadToken(core::ptr::null(), core::ptr::null_mut());
        say!(t, "SetThreadToken(NULL, NULL): {}", r);
        thread_token_line(t, "after SetThreadToken(NULL, NULL)");
        let r = ImpersonateNamedPipeClient(pipe);
        say!(t, "impersonate again: {}", r);
        let r = RevertToSelf();
        say!(t, "RevertToSelf: {}", r);
        thread_token_line(t, "after RevertToSelf");

        // the thread that did not read the byte impersonates (the pipe's state is shared)
        run_other(false);
        say!(
            t,
            "second thread, byte already read by the main thread: impersonated {} (error {}) type {} level {}",
            OTHER_IMP,
            OTHER_ERR,
            OTHER_TYPE,
            OTHER_LEVEL
        );
        c.finish();
        DisconnectNamedPipe(pipe);

        // a second client on the same instance: connect, then another thread reads and impersonates
        let c2 = Client::start("Th1", 0, true, 0);
        ConnectNamedPipe(pipe, core::ptr::null_mut());
        c2.up();
        SetLastError(0x5A5A);
        let r = ImpersonateNamedPipeClient(pipe);
        say!(t, "second client connected, byte not read yet: {} (error {})", r, GetLastError());
        run_other(true);
        say!(
            t,
            "another thread read the byte and impersonated: read {} impersonated {} (error {}) type {} level {}",
            OTHER_READ,
            OTHER_IMP,
            OTHER_ERR,
            OTHER_TYPE,
            OTHER_LEVEL
        );
        let r = ImpersonateNamedPipeClient(pipe);
        say!(t, "main thread after that: {}", r);
        RevertToSelf();
        c2.finish();
        CloseHandle(pipe);
    }
}
