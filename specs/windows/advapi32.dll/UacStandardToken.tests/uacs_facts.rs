#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, unused_imports, clippy::all)]

// The declarations in this file are from Microsoft Learn (prototypes) and the
// public SDK headers named beside them (constants, layouts); the helpers are
// this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
type HANDLE = *mut c_void;
type DWORD = u32;

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

/// A NUL-terminated UTF-16 copy of a string (up to 299 units; no characters outside the BMP).
fn wide(s: &str) -> [u16; 300] {
    let mut w = [0u16; 300];
    for (i, c) in s.chars().enumerate().take(299) {
        w[i] = c as u32 as u16;
    }
    w
}

/// UTF-16 units as text: printable ASCII as it is, anything else as `<xxxx>`.
fn units_text(u: &[u16]) -> Buf<700> {
    let mut b = Buf::<700>::new();
    for &c in u {
        if (0x20..0x7f).contains(&c) {
            let _ = write!(b, "{}", c as u8 as char);
        } else {
            let _ = write!(b, "<{c:04x}>");
        }
    }
    b
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

/// Whether two UTF-16 strings are equal without regard to ASCII case.
fn eq_nocase(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(&x, &y)| {
            let l = |c: u16| if (0x41..=0x5a).contains(&c) { c + 32 } else { c };
            l(x) == l(y)
        })
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
}

fn gle() -> i64 {
    // SAFETY: plain call.
    i64::from(unsafe { GetLastError() })
}

fn clear_error() {
    // SAFETY: plain call.
    unsafe { SetLastError(0xDEAD) };
}

/// The last error after a failed call, else -2 (success leaves it unspecified).
fn gle_if(ok: BOOL) -> i64 {
    if ok == 0 { gle() } else { -2 }
}
// recorded with the standard token (poly-oracle.ps1 -Token standard)

// ---------------------------------------------------------------- group E helpers: tokens, child processes

const TOKEN_QUERY: u32 = 0x8;
const TOKEN_DUPLICATE: u32 = 0x2;
const TOKEN_IMPERSONATE: u32 = 0x4;
const INFINITE: u32 = 0xFFFF_FFFF;

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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleFileNameW(module: HANDLE, buf: *mut u16, size: u32) -> u32;
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
    fn TerminateProcess(h: HANDLE, code: u32) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    fn GetTokenInformation(token: HANDLE, class: u32, info: *mut c_void, len: u32, ret_len: *mut u32) -> BOOL;
    fn CheckTokenMembership(token: HANDLE, sid: *const c_void, is_member: *mut BOOL) -> BOOL;
    fn LookupPrivilegeNameW(system: *const u16, luid: *const u64, name: *mut u16, len: *mut u32) -> BOOL;
}

/// S-1-5-32-544, BUILTIN\Administrators.
const ADMINS_SID: [u8; 16] = [1, 2, 0, 0, 0, 0, 0, 5, 0x20, 0, 0, 0, 0x20, 2, 0, 0];
/// S-1-5-32-545, BUILTIN\Users.
const USERS_SID: [u8; 16] = [1, 2, 0, 0, 0, 0, 0, 5, 0x20, 0, 0, 0, 0x21, 2, 0, 0];

fn own_token(access: u32) -> HANDLE {
    let mut tok: HANDLE = core::ptr::null_mut();
    // SAFETY: valid out pointer; the pseudo handle of the current process.
    unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut tok) };
    tok
}

/// A 4-byte class value, or -(1000 + error) when the call fails.
fn token_u32(tok: HANDLE, class: u32) -> i64 {
    let mut v = [0u8; 4];
    let mut r = 0u32;
    // SAFETY: valid 4-byte buffer.
    let ok = unsafe { GetTokenInformation(tok, class, v.as_mut_ptr().cast(), 4, &mut r) };
    if ok == 0 { -1000 - gle() } else { i64::from(u32::from_le_bytes(v)) }
}

/// The token's integrity RID (0x1000, 0x2000, 0x3000, 0x4000), 0 if unknown.
fn token_il(tok: HANDLE) -> u32 {
    let mut buf = [0u8; 128];
    let mut r = 0u32;
    // SAFETY: valid buffer.
    let ok = unsafe { GetTokenInformation(tok, 25, buf.as_mut_ptr().cast(), 128, &mut r) };
    if ok == 0 {
        return 0;
    }
    let cnt = buf[17] as usize;
    let q = 16 + 8 + 4 * (cnt - 1);
    u32::from_le_bytes([buf[q], buf[q + 1], buf[q + 2], buf[q + 3]])
}

/// The attributes of the group `sid` in the token's TokenGroups (None: not in the list).
fn group_attr(tok: HANDLE, sid: &[u8; 16]) -> Option<u32> {
    let mut buf = [0u8; 2048];
    let mut r = 0u32;
    // SAFETY: valid buffer.
    let ok = unsafe { GetTokenInformation(tok, 2, buf.as_mut_ptr().cast(), 2048, &mut r) };
    if ok == 0 {
        return None;
    }
    let n = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    let base = buf.as_ptr() as usize;
    for i in 0..n {
        let e = 8 + 16 * i;
        let p = usize::from_le_bytes([buf[e], buf[e + 1], buf[e + 2], buf[e + 3], buf[e + 4], buf[e + 5], buf[e + 6], buf[e + 7]]);
        let off = p - base;
        if &buf[off..off + 16] == sid {
            return Some(u32::from_le_bytes([buf[e + 8], buf[e + 9], buf[e + 10], buf[e + 11]]));
        }
    }
    None
}

fn group_count(tok: HANDLE) -> i64 {
    let mut buf = [0u8; 2048];
    let mut r = 0u32;
    // SAFETY: valid buffer.
    let ok = unsafe { GetTokenInformation(tok, 2, buf.as_mut_ptr().cast(), 2048, &mut r) };
    if ok == 0 { -1 } else { i64::from(u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]])) }
}

/// The privileges of the token: (name, attributes), sorted by name, up to 40.
fn privileges(tok: HANDLE) -> ([([u16; 64], usize, u32); 40], usize) {
    let mut out = [([0u16; 64], 0usize, 0u32); 40];
    let mut buf = [0u8; 1024];
    let mut r = 0u32;
    // SAFETY: valid buffer.
    let ok = unsafe { GetTokenInformation(tok, 3, buf.as_mut_ptr().cast(), 1024, &mut r) };
    if ok == 0 {
        return (out, 0);
    }
    let n = (u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize).min(40);
    for i in 0..n {
        let e = 4 + 12 * i;
        let mut luid = [0u8; 8];
        luid.copy_from_slice(&buf[e..e + 8]);
        let luid = u64::from_le_bytes(luid);
        let attr = u32::from_le_bytes([buf[e + 8], buf[e + 9], buf[e + 10], buf[e + 11]]);
        let mut len = 64u32;
        // SAFETY: valid buffers.
        let ok = unsafe { LookupPrivilegeNameW(core::ptr::null(), &luid, out[i].0.as_mut_ptr(), &mut len) };
        out[i].1 = if ok != 0 { len as usize } else { 0 };
        out[i].2 = attr;
    }
    // insertion sort by name
    for i in 1..n {
        let mut j = i;
        while j > 0 && out[j - 1].0[..out[j - 1].1] > out[j].0[..out[j].1] {
            out.swap(j - 1, j);
            j -= 1;
        }
    }
    (out, n)
}

fn print_privileges(t: &mut Test, label: &str, tok: HANDLE) {
    let (p, n) = privileges(tok);
    say!(t, "{label}: privilege count {n}");
    for e in &p[..n] {
        say!(t, "{label}: privilege {} attr {:#x}", units_text(&e.0[..e.1]).as_str(), e.2);
    }
}

/// One line with the facts of a token: elevation, type, restrictions, integrity, admin group, groups.
fn token_summary(t: &mut Test, label: &str, tok: HANDLE) {
    let admin = match group_attr(tok, &ADMINS_SID) {
        Some(a) => a as i64,
        None => -1,
    };
    say!(
        t,
        "{label}: type {} elevation {} elevationtype {} restricted {} integrity {:#x} admins attr {:#x}",
        token_u32(tok, 8),
        token_u32(tok, 20),
        token_u32(tok, 18),
        token_u32(tok, 21),
        token_il(tok),
        admin
    );
}

/// The code a child process ends with to tell its parent which token it has:
/// 0x10000 | elevation | elevation type << 1 | integrity (RID >> 12) << 4 | admins deny-only << 8 | admins enabled << 9 | linked token present << 10.
fn token_code() -> u32 {
    let tok = own_token(TOKEN_QUERY);
    let el = token_u32(tok, 20) as u32 & 1;
    let ty = token_u32(tok, 18) as u32 & 3;
    let il = (token_il(tok) >> 12) & 7;
    let (deny, on) = match group_attr(tok, &ADMINS_SID) {
        Some(a) => (u32::from(a & 0x10 != 0), u32::from(a & 4 != 0)),
        None => (0, 0),
    };
    let mut h = [0u8; 8];
    let mut r = 0u32;
    // SAFETY: valid buffer.
    let linked = unsafe { GetTokenInformation(tok, 19, h.as_mut_ptr().cast(), 8, &mut r) };
    0x10000 | el | (ty << 1) | (il << 4) | (deny << 8) | (on << 9) | (u32::from(linked != 0) << 10)
}

fn describe_code(code: u32) -> Buf<200> {
    let mut b = Buf::<200>::new();
    if code & 0x10000 == 0 {
        let _ = write!(b, "no token report, exit code {code:#x}");
    } else {
        let _ = write!(
            b,
            "elevation {} type {} integrity {} admins-deny-only {} admins-enabled {} linked-present {}",
            code & 1,
            (code >> 1) & 3,
            (code >> 4) & 7,
            (code >> 8) & 1,
            (code >> 9) & 1,
            (code >> 10) & 1
        );
    }
    b
}

/// The text after ` --child ` on this process's command line (up to 40 ASCII characters).
fn child_arg() -> Option<Buf<40>> {
    // SAFETY: the command line is a NUL-terminated string owned by the process.
    let cl = wstr_at(unsafe { GetCommandLineW() }, 600);
    let key = b" --child ";
    let mut i = 0;
    while i + key.len() <= cl.len() {
        if key.iter().enumerate().all(|(k, &c)| cl[i + k] == u16::from(c)) {
            let mut b = Buf::<40>::new();
            for &c in &cl[i + key.len()..] {
                let _ = write!(b, "{}", (c as u8) as char);
            }
            return Some(b);
        }
        i += 1;
    }
    None
}

/// This image's path (UTF-16, NUL-terminated) in a buffer; the length without the NUL.
fn self_path() -> ([u16; 300], usize) {
    let mut p = [0u16; 300];
    // SAFETY: valid buffer.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), p.as_mut_ptr(), 299) } as usize;
    (p, n)
}

/// The command line `"<this image>" --child <arg>` as a NUL-terminated buffer.
fn child_cmdline(arg: &str) -> [u16; 600] {
    let (p, n) = self_path();
    let mut c = [0u16; 600];
    let mut k = 0;
    c[k] = b'"' as u16;
    k += 1;
    for &u in &p[..n] {
        c[k] = u;
        k += 1;
    }
    c[k] = b'"' as u16;
    k += 1;
    for b in " --child ".bytes().chain(arg.bytes()) {
        c[k] = u16::from(b);
        k += 1;
    }
    c
}

/// Start this image as `--child <arg>`, wait for it (20 s at most) and return its exit code, or None when CreateProcessW failed
/// (the error is then in `gle()`).
fn run_self_child(arg: &str) -> Option<u32> {
    let (path, _) = self_path();
    let mut cmd = child_cmdline(arg);
    let mut si = StartupInfoW { cb: 104, pad0: 0, rest: [0; 12] };
    let mut pi = ProcInfo { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), pid: 0, tid: 0 };
    // SAFETY: valid, NUL-terminated buffers; zeroed STARTUPINFOW of 104 bytes.
    let ok = unsafe {
        CreateProcessW(path.as_ptr(), cmd.as_mut_ptr(), core::ptr::null(), core::ptr::null(), 0, 0, core::ptr::null(), core::ptr::null(), &mut si, &mut pi)
    };
    if ok == 0 {
        return None;
    }
    let mut code = 0xDEAD_DEAD;
    // SAFETY: handles from CreateProcessW.
    unsafe {
        if WaitForSingleObject(pi.process, 20000) != 0 {
            TerminateProcess(pi.process, 0xDEAD);
        } else {
            GetExitCodeProcess(pi.process, &mut code);
        }
        CloseHandle(pi.process);
        CloseHandle(pi.thread);
    }
    Some(code)
}

/// The child half of a test: `--child <arg>` runs `$child(arg) -> exit code` and never opens the observation file;
/// otherwise the test runs as usual.
macro_rules! main_with_child {
    ($child:path, $test:path) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn mainCRTStartup() -> ! {
            if let Some(a) = child_arg() {
                exit($child(a.as_str()))
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

/// The standard child of the token tests: mode "token" reports the token.
fn token_child(arg: &str) -> u32 {
    match arg {
        "token" => token_code(),
        _ => 0x77,
    }
}

// ---------------------------------------------------------------- file helpers (scratch directory beside the image)

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CopyFileW(from: *const u16, to: *const u16, fail_if_exists: BOOL) -> BOOL;
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *const c_void, disp: u32, flags: u32, tmpl: HANDLE) -> HANDLE;
    fn WriteFile(h: HANDLE, buf: *const u8, n: u32, written: *mut u32, ov: *mut c_void) -> BOOL;
    fn DeleteFileW(name: *const u16) -> BOOL;
}

/// The path of a file beside this image: the image's directory, then `name`.
fn beside(name: &str) -> [u16; 300] {
    let (p, n) = self_path();
    let mut last = 0;
    for (i, &u) in p[..n].iter().enumerate() {
        if u == b'\\' as u16 {
            last = i + 1;
        }
    }
    let mut out = [0u16; 300];
    out[..last].copy_from_slice(&p[..last]);
    for (i, b) in name.bytes().enumerate() {
        out[last + i] = u16::from(b);
    }
    out
}

fn write_file(path: &[u16; 300], text: &str) -> bool {
    // SAFETY: valid NUL-terminated path; CREATE_ALWAYS 2, GENERIC_WRITE 0x40000000.
    let h = unsafe { CreateFileW(path.as_ptr(), 0x4000_0000, 0, core::ptr::null(), 2, 0x80, core::ptr::null_mut()) };
    if h as isize == -1 {
        return false;
    }
    let mut w = 0u32;
    // SAFETY: valid buffer and handle.
    let ok = unsafe { WriteFile(h, text.as_ptr(), text.len() as u32, &mut w, core::ptr::null_mut()) };
    // SAFETY: handle from CreateFileW.
    unsafe { CloseHandle(h) };
    ok != 0
}

fn manifest(level: &str) -> Buf<600> {
    let mut b = Buf::<600>::new();
    let _ = write!(
        b,
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<assembly xmlns=\"urn:schemas-microsoft-com:asm.v1\" manifestVersion=\"1.0\"><trustInfo xmlns=\"urn:schemas-microsoft-com:asm.v3\"><security><requestedPrivileges><requestedExecutionLevel level=\"{level}\" uiAccess=\"false\"/></requestedPrivileges></security></trustInfo></assembly>"
    );
    b
}


// ---------------------------------------------------------------- ShellExecute helpers

#[repr(C)]
struct ShellExecuteInfoW {
    cb_size: u32,
    mask: u32,
    hwnd: HANDLE,
    verb: *const u16,
    file: *const u16,
    params: *const u16,
    dir: *const u16,
    show: i32,
    inst_app: usize,
    id_list: *mut c_void,
    class: *const u16,
    hkey_class: HANDLE,
    hot_key: u32,
    monitor: HANDLE,
    process: HANDLE,
}

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ShellExecuteExW(info: *mut ShellExecuteInfoW) -> BOOL;
    fn ShellExecuteW(hwnd: HANDLE, verb: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> usize;
    fn ShellExecuteA(hwnd: HANDLE, verb: *const u8, file: *const u8, params: *const u8, dir: *const u8, show: i32) -> usize;
}

const SEE_MASK_NOCLOSEPROCESS: u32 = 0x40;
const SEE_MASK_NOASYNC: u32 = 0x100;
const SEE_MASK_FLAG_NO_UI: u32 = 0x400;

/// ShellExecuteExW on this image (or `file`) with `--child token`; reports the call's results.
fn exec_ex(t: &mut Test, label: &str, verb: Option<&str>, file: &[u16; 300], mask: u32) {
    let v = wide(verb.unwrap_or(""));
    let params = wide("--child token");
    let mut info = ShellExecuteInfoW {
        cb_size: core::mem::size_of::<ShellExecuteInfoW>() as u32,
        mask,
        hwnd: core::ptr::null_mut(),
        verb: if verb.is_some() { v.as_ptr() } else { core::ptr::null() },
        file: file.as_ptr(),
        params: params.as_ptr(),
        dir: core::ptr::null(),
        show: 0,
        inst_app: 0xAAAA,
        id_list: core::ptr::null_mut(),
        class: core::ptr::null(),
        hkey_class: core::ptr::null_mut(),
        hot_key: 0,
        monitor: core::ptr::null_mut(),
        process: core::ptr::null_mut(),
    };
    clear_error();
    // SAFETY: valid struct and strings.
    let ok = unsafe { ShellExecuteExW(&mut info) };
    let err = gle_if(ok);
    let started = !info.process.is_null();
    let mut tail = Buf::<200>::new();
    if started {
        let mut code = 0u32;
        // SAFETY: the process handle returned by ShellExecuteExW.
        unsafe {
            WaitForSingleObject(info.process, 20000);
            GetExitCodeProcess(info.process, &mut code);
            CloseHandle(info.process);
        }
        let _ = write!(tail, "child {}", describe_code(code).as_str());
    }
    let inst = info.inst_app;
    say!(
        t,
        "{label}: ok {ok} err {err} hInstApp {} hProcess {} {}",
        if inst > 32 { "success(>32)" } else if inst == 0xAAAA { "unchanged" } else { "error" },
        if started { "set" } else { "null" },
        tail.as_str()
    );
    if ok == 0 && inst != 0xAAAA && inst <= 32 {
        say!(t, "{label}: hInstApp error code {inst}");
    }
}


main_with_child!(token_child, test);

fn test(t: &mut Test) {
    let tok = own_token(TOKEN_QUERY | TOKEN_DUPLICATE);
    token_summary(t, "own token", tok);
    say!(t, "own token: mandatory policy {} uiaccess {} sandboxinert {} appcontainer {}", token_u32(tok, 27), token_u32(tok, 26), token_u32(tok, 15), token_u32(tok, 29));
    say!(t, "own token: virtualization allowed {} enabled {}", token_u32(tok, 23), token_u32(tok, 24));
    // the groups the Administrators and Users aliases have in this token
    for (n, sid) in [("Administrators", &ADMINS_SID), ("Users", &USERS_SID)] {
        match group_attr(tok, sid) {
            Some(a) => say!(t, "own token: group {n} attributes {a:#x}"),
            None => say!(t, "own token: group {n} absent"),
        }
    }
    say!(t, "own token: groups more than 8: {}", group_count(tok) > 8);
    print_privileges(t, "own token", tok);

    // class 20 and 18 with the wrong length
    let mut v = [0u8; 8];
    let mut r = 0u32;
    clear_error();
    // SAFETY: valid buffers.
    let ok = unsafe { GetTokenInformation(tok, 20, v.as_mut_ptr().cast(), 8, &mut r) };
    say!(t, "TokenElevation with 8 bytes: ok {ok} err {} returned length {r}", gle_if(ok));
    r = 0;
    clear_error();
    // SAFETY: valid buffers.
    let ok = unsafe { GetTokenInformation(tok, 18, v.as_mut_ptr().cast(), 2, &mut r) };
    say!(t, "TokenElevationType with 2 bytes: ok {ok} err {} returned length {r}", gle_if(ok));

    // a primary copy and an impersonation copy keep the split-token facts
    for (label, level, kind) in [("impersonation copy", 2u32, 2u32), ("primary copy", 2, 1)] {
        let mut dup: HANDLE = core::ptr::null_mut();
        // SAFETY: valid out pointer.
        let ok = unsafe { DuplicateTokenEx(tok, 0xF01FF, core::ptr::null(), level, kind, &mut dup) };
        say!(t, "{label}: ok {ok}");
        if ok != 0 {
            token_summary(t, label, dup);
            // SAFETY: handle from DuplicateTokenEx.
            unsafe { CloseHandle(dup) };
        }
    }
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DuplicateTokenEx(token: HANDLE, access: u32, attrs: *const c_void, level: u32, kind: u32, out: *mut HANDLE) -> BOOL;
}
