//! `a32test.exe` (ADR 0083): the program the kernel's ring-3 test
//! `nt_advapi32_program_runs_through_forwarders_to_kernelbase` runs. It
//! imports from `advapi32.dll` (whose registry, token and security
//! functions forward to `kernelbase` through API-set contracts) and
//! `kernel32.dll`, and checks the documented behaviour of the registry API,
//! the single user's token, SIDs, ACLs and security descriptors (Learn's
//! pages).
//!
//! It exits through `ExitProcess(PASS)` when every check passed; a failed
//! check prints `a32test: step N failed (got G)` on its standard output and
//! exits with `0x80 + N`.

#![no_std]
#![no_main]
#![allow(clippy::missing_safety_doc)]

use core::sync::atomic::{AtomicU32, Ordering};
use poly_win32_rt as _;

type Handle = isize;
type HKey = isize;
const INVALID_HANDLE_VALUE: Handle = -1;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ExitProcess(code: u32) -> !;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn GetStdHandle(which: u32) -> Handle;
    fn WriteFile(h: Handle, buf: *const u8, len: u32, written: *mut u32, ov: *mut u8) -> i32;
    fn CloseHandle(h: Handle) -> i32;
    fn GetCurrentProcess() -> Handle;
    fn GetCurrentThread() -> Handle;
    fn LocalFree(p: *mut u8) -> *mut u8;
    fn SetEnvironmentVariableW(name: *const u16, value: *const u16) -> i32;
    fn GetModuleHandleW(name: *const u16) -> *mut u8;
    fn GetProcAddress(m: *mut u8, name: *const u8) -> *mut u8;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegOpenKeyExW(hkey: HKey, sub: *const u16, options: u32, sam: u32, out: *mut HKey) -> u32;
    fn RegOpenKeyExA(hkey: HKey, sub: *const u8, options: u32, sam: u32, out: *mut HKey) -> u32;
    fn RegCreateKeyExW(
        hkey: HKey,
        sub: *const u16,
        reserved: u32,
        class: *const u16,
        options: u32,
        sam: u32,
        sa: *const u8,
        out: *mut HKey,
        disposition: *mut u32,
    ) -> u32;
    fn RegQueryValueExW(
        hkey: HKey,
        name: *const u16,
        r: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> u32;
    fn RegQueryValueExA(hkey: HKey, name: *const u8, r: *mut u32, kind: *mut u32, data: *mut u8, size: *mut u32)
    -> u32;
    fn RegSetValueExW(hkey: HKey, name: *const u16, r: u32, kind: u32, data: *const u8, size: u32) -> u32;
    fn RegSetValueExA(hkey: HKey, name: *const u8, r: u32, kind: u32, data: *const u8, size: u32) -> u32;
    fn RegGetValueW(
        hkey: HKey,
        sub: *const u16,
        name: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> u32;
    fn RegEnumKeyExW(
        hkey: HKey,
        index: u32,
        name: *mut u16,
        len: *mut u32,
        r: *mut u32,
        class: *mut u16,
        class_len: *mut u32,
        time: *mut u64,
    ) -> u32;
    fn RegEnumValueW(
        hkey: HKey,
        index: u32,
        name: *mut u16,
        len: *mut u32,
        r: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> u32;
    fn RegQueryInfoKeyW(
        hkey: HKey,
        class: *mut u16,
        class_len: *mut u32,
        r: *mut u32,
        subkeys: *mut u32,
        max_subkey: *mut u32,
        max_class: *mut u32,
        values: *mut u32,
        max_value_name: *mut u32,
        max_value: *mut u32,
        sd: *mut u32,
        time: *mut u64,
    ) -> u32;
    fn RegDeleteKeyW(hkey: HKey, sub: *const u16) -> u32;
    fn RegDeleteValueW(hkey: HKey, name: *const u16) -> u32;
    fn RegCloseKey(hkey: HKey) -> u32;

    fn OpenProcessToken(process: Handle, access: u32, out: *mut Handle) -> i32;
    fn OpenThreadToken(thread: Handle, access: u32, as_self: i32, out: *mut Handle) -> i32;
    fn GetTokenInformation(token: Handle, class: u32, info: *mut u8, len: u32, ret: *mut u32) -> i32;
    fn CheckTokenMembership(token: Handle, sid: *const u8, member: *mut i32) -> i32;
    fn GetUserNameW(buf: *mut u16, size: *mut u32) -> i32;
    fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut u64) -> i32;
    fn AdjustTokenPrivileges(
        token: Handle,
        disable_all: i32,
        new_state: *const u8,
        length: u32,
        previous: *mut u8,
        ret: *mut u32,
    ) -> i32;

    fn AllocateAndInitializeSid(
        auth: *const [u8; 6],
        count: u8,
        s0: u32,
        s1: u32,
        s2: u32,
        s3: u32,
        s4: u32,
        s5: u32,
        s6: u32,
        s7: u32,
        out: *mut *mut u8,
    ) -> i32;
    fn FreeSid(sid: *mut u8) -> *mut u8;
    fn EqualSid(a: *const u8, b: *const u8) -> i32;
    fn GetLengthSid(sid: *const u8) -> u32;
    fn IsValidSid(sid: *const u8) -> i32;
    fn CopySid(len: u32, dest: *mut u8, src: *const u8) -> i32;
    fn GetSidSubAuthority(sid: *const u8, n: u32) -> *mut u32;
    fn GetSidSubAuthorityCount(sid: *const u8) -> *mut u8;
    fn ConvertSidToStringSidW(sid: *const u8, out: *mut *mut u16) -> i32;
    fn ConvertStringSidToSidW(s: *const u16, out: *mut *mut u8) -> i32;

    fn InitializeSecurityDescriptor(sd: *mut u8, revision: u32) -> i32;
    fn IsValidSecurityDescriptor(sd: *const u8) -> i32;
    fn SetSecurityDescriptorDacl(sd: *mut u8, present: i32, acl: *mut u8, defaulted: i32) -> i32;
    fn GetSecurityDescriptorDacl(sd: *const u8, present: *mut i32, acl: *mut *mut u8, defaulted: *mut i32) -> i32;
    fn SetSecurityDescriptorOwner(sd: *mut u8, sid: *mut u8, defaulted: i32) -> i32;
    fn GetSecurityDescriptorOwner(sd: *const u8, sid: *mut *mut u8, defaulted: *mut i32) -> i32;
    fn GetSecurityDescriptorLength(sd: *const u8) -> u32;
    fn MakeSelfRelativeSD(abs: *const u8, rel: *mut u8, len: *mut u32) -> i32;
    fn InitializeAcl(acl: *mut u8, len: u32, revision: u32) -> i32;
    fn IsValidAcl(acl: *mut u8) -> i32;
    fn AddAccessAllowedAce(acl: *mut u8, revision: u32, mask: u32, sid: *const u8) -> i32;
    fn AddAccessDeniedAce(acl: *mut u8, revision: u32, mask: u32, sid: *const u8) -> i32;
    fn GetAce(acl: *mut u8, index: u32, ace: *mut *mut u8) -> i32;
    fn DeleteAce(acl: *mut u8, index: u32) -> i32;
    fn GetAclInformation(acl: *mut u8, info: *mut u8, len: u32, class: u32) -> i32;
}

/// The exit status when every check passed.
const PASS: u32 = 0x41;

/// A NUL-terminated UTF-16 copy of an ASCII literal.
macro_rules! w {
    ($s:literal) => {{
        const S: &[u8] = $s.as_bytes();
        const W: [u16; S.len() + 1] = {
            let mut o = [0u16; S.len() + 1];
            let mut i = 0;
            while i < S.len() {
                o[i] = S[i] as u16;
                i += 1;
            }
            o
        };
        W.as_ptr()
    }};
}

/// A NUL-terminated ASCII literal.
macro_rules! a {
    ($s:literal) => {
        concat!($s, "\0").as_ptr()
    };
}

const fn hkey(v: u32) -> HKey {
    v as i32 as HKey
}
const HKEY_CLASSES_ROOT: HKey = hkey(0x8000_0000);
const HKEY_CURRENT_USER: HKey = hkey(0x8000_0001);
const HKEY_LOCAL_MACHINE: HKey = hkey(0x8000_0002);
const HKEY_USERS: HKey = hkey(0x8000_0003);

const KEY_READ: u32 = 0x2_0019;
const KEY_ALL_ACCESS: u32 = 0xF_003F;
const REG_SZ: u32 = 1;
const REG_EXPAND_SZ: u32 = 2;
const REG_BINARY: u32 = 3;
const REG_DWORD: u32 = 4;
const REG_MULTI_SZ: u32 = 7;
const REG_QWORD: u32 = 11;
const REG_CREATED_NEW_KEY: u32 = 1;
const REG_OPENED_EXISTING_KEY: u32 = 2;
const RRF_RT_REG_SZ: u32 = 0x2;
const RRF_RT_REG_EXPAND_SZ: u32 = 0x4;
const RRF_RT_DWORD: u32 = 0x18;
const RRF_RT_QWORD: u32 = 0x48;
const RRF_RT_ANY: u32 = 0xFFFF;
const RRF_NOEXPAND: u32 = 0x1000_0000;
const RRF_ZEROONFAILURE: u32 = 0x2000_0000;

const TOKEN_QUERY: u32 = 8;
const TOKEN_USER: u32 = 1;
const TOKEN_GROUPS: u32 = 2;
const TOKEN_ELEVATION_TYPE: u32 = 18;
const TOKEN_ELEVATION: u32 = 20;
const TOKEN_INTEGRITY_LEVEL: u32 = 25;
const SECURITY_NT_AUTHORITY: [u8; 6] = [0, 0, 0, 0, 0, 5];
const SECURITY_WORLD_SID_AUTHORITY: [u8; 6] = [0, 0, 0, 0, 0, 1];
const GENERIC_ALL: u32 = 0x1000_0000;
const ACL_REVISION: u32 = 2;
const ACL_SIZE_INFORMATION: u32 = 2;

const ERROR_SUCCESS: u32 = 0;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_MORE_DATA: u32 = 234;
const ERROR_NO_MORE_ITEMS: u32 = 259;
const ERROR_NO_TOKEN: u32 = 1008;
const ERROR_UNKNOWN_REVISION: u32 = 1305;
const ERROR_INVALID_SID: u32 = 1337;
const ERROR_INVALID_SECURITY_DESCR: u32 = 1338;
const ERROR_ALLOTTED_SPACE_EXCEEDED: u32 = 1344;
const ERROR_UNSUPPORTED_TYPE: u32 = 1630;
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;

// ----- reporting ---------------------------------------------------------------------

fn out(bytes: &[u8]) {
    // SAFETY: kernel32 calls with valid buffers.
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        if h != 0 && h != INVALID_HANDLE_VALUE {
            let mut n = 0;
            WriteFile(h, bytes.as_ptr(), bytes.len() as u32, &mut n, core::ptr::null_mut());
        }
    }
}

fn out_num(mut v: u32) {
    let mut b = [0u8; 10];
    let mut i = b.len();
    loop {
        i -= 1;
        b[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    out(&b[i..]);
}

static STEP: AtomicU32 = AtomicU32::new(0);

/// Check `ok` as the next step, reporting `got` on failure.
fn check_got(ok: bool, got: u32) {
    let step = STEP.fetch_add(1, Ordering::Relaxed) + 1;
    if !ok {
        out(b"a32test: step ");
        out_num(step);
        out(b" failed (got ");
        out_num(got);
        out(b")\n");
        // SAFETY: never returns.
        unsafe { ExitProcess(0x80 + (step & 0x7F)) }
    }
}

/// Check `ok`, reporting the last error on failure.
fn check(ok: bool) {
    // SAFETY: a plain kernel32 call.
    let e = unsafe { GetLastError() };
    check_got(ok, e);
}

/// Check a returned (registry) error code.
fn check_code(got: u32, want: u32) {
    check_got(got == want, got);
}

/// Check that a call failed with `want` as the last error.
fn check_fail(r: i32, want: u32) {
    // SAFETY: a plain kernel32 call.
    let e = unsafe { GetLastError() };
    check_got(r == 0 && e == want, e);
}

fn wlen(p: *const u16) -> usize {
    let mut n = 0;
    // SAFETY: NUL-terminated strings.
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    n
}

fn wide_eq(p: *const u16, a: &[u8]) -> bool {
    let n = wlen(p);
    // SAFETY: `n` units are readable.
    n == a.len() && (0..n).all(|i| unsafe { *p.add(i) } == u16::from(a[i]))
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

// ----- the checks --------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn a32test_main(_peb: *mut u8) -> u32 {
    out(b"a32test: start\n");
    // SAFETY: every call is an advapi32/kernel32 export with valid arguments.
    unsafe {
        forwarding();
        registry_read();
        registry_write();
        classes_root();
        token();
        sids();
        descriptors();
        out(b"a32test: all ");
        out_num(STEP.load(Ordering::Relaxed));
        out(b" checks passed\n");
        ExitProcess(PASS)
    }
}

/// `GetProcAddress` through `advapi32`'s forwarder lands in `kernelbase`;
/// its own functions are its own.
unsafe fn forwarding() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let adv = GetModuleHandleW(w!("advapi32.dll"));
        let kb = GetModuleHandleW(w!("kernelbase.dll"));
        check(!adv.is_null() && !kb.is_null());
        let fwd = GetProcAddress(adv, a!("RegOpenKeyExW"));
        check(!fwd.is_null() && fwd == GetProcAddress(kb, a!("RegOpenKeyExW")));
        let own = GetProcAddress(adv, a!("ConvertSidToStringSidW"));
        check(!own.is_null() && GetProcAddress(kb, a!("ConvertSidToStringSidW")).is_null());
    }
}

unsafe fn registry_read() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let mut k: HKey = 0;
        check_code(
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
                0,
                KEY_READ,
                &mut k,
            ),
            ERROR_SUCCESS,
        );
        // Size first, then the data.
        let (mut kind, mut size) = (0u32, 0u32);
        check_code(
            RegQueryValueExW(k, w!("CurrentBuild"), core::ptr::null_mut(), &mut kind, core::ptr::null_mut(), &mut size),
            0,
        );
        check_got(kind == REG_SZ && size >= 10, size);
        let mut buf = [0u16; 32];
        let mut cap = 64u32;
        check_code(
            RegQueryValueExW(
                k,
                w!("CurrentBuild"),
                core::ptr::null_mut(),
                &mut kind,
                buf.as_mut_ptr().cast(),
                &mut cap,
            ),
            0,
        );
        check(buf[..5] == [0x31, 0x39, 0x30, 0x34, 0x35]);
        // Too small: ERROR_MORE_DATA and the size.
        let mut small = 2u32;
        check_code(
            RegQueryValueExW(
                k,
                w!("CurrentBuild"),
                core::ptr::null_mut(),
                &mut kind,
                buf.as_mut_ptr().cast(),
                &mut small,
            ),
            ERROR_MORE_DATA,
        );
        check_got(small == size, small);
        // A DWORD.
        let mut major = 0u32;
        let mut four = 4u32;
        check_code(
            RegQueryValueExW(
                k,
                w!("CurrentMajorVersionNumber"),
                core::ptr::null_mut(),
                &mut kind,
                (&raw mut major).cast(),
                &mut four,
            ),
            0,
        );
        check_got(kind == REG_DWORD && major == 10, major);
        // A missing value, a reserved pointer, a buffer without its size.
        check_code(
            RegQueryValueExW(k, w!("NoSuchValue"), core::ptr::null_mut(), &mut kind, core::ptr::null_mut(), &mut size),
            ERROR_FILE_NOT_FOUND,
        );
        check_code(
            RegQueryValueExW(k, w!("CurrentBuild"), &mut size, &mut kind, core::ptr::null_mut(), &mut size),
            ERROR_INVALID_PARAMETER,
        );
        check_code(
            RegQueryValueExW(
                k,
                w!("CurrentBuild"),
                core::ptr::null_mut(),
                &mut kind,
                buf.as_mut_ptr().cast(),
                core::ptr::null_mut(),
            ),
            ERROR_INVALID_PARAMETER,
        );
        // The ANSI form converts strings and their sizes.
        let mut abuf = [0u8; 16];
        let mut asize = 16u32;
        check_code(
            RegQueryValueExA(k, a!("ProductName"), core::ptr::null_mut(), &mut kind, abuf.as_mut_ptr(), &mut asize),
            0,
        );
        check_got(kind == REG_SZ && &abuf[..4] == b"Poly" && (4..=5).contains(&asize), asize);
        check_code(RegCloseKey(k), 0);
        // RegGetValueW with a subkey, NUL-terminated.
        let mut gsize = 64u32;
        check_code(
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
                w!("ProductName"),
                RRF_RT_REG_SZ,
                &mut kind,
                buf.as_mut_ptr().cast(),
                &mut gsize,
            ),
            0,
        );
        check_got(gsize == 10 && wide_eq(buf.as_ptr(), b"Poly"), gsize);
        // A type outside the restriction.
        check_code(
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
                w!("ProductName"),
                RRF_RT_DWORD,
                &mut kind,
                buf.as_mut_ptr().cast(),
                &mut gsize,
            ),
            ERROR_UNSUPPORTED_TYPE,
        );
        // Missing keys; an empty subkey of a predefined key is that key.
        check_code(
            RegOpenKeyExW(HKEY_LOCAL_MACHINE, w!("SOFTWARE\\NoSuchKeyA32"), 0, KEY_READ, &mut k),
            ERROR_FILE_NOT_FOUND,
        );
        check_code(RegOpenKeyExW(HKEY_LOCAL_MACHINE, w!(""), 0, KEY_READ, &mut k), 0);
        check(k == HKEY_LOCAL_MACHINE);
        check_code(RegOpenKeyExA(HKEY_USERS, a!(".DEFAULT\\Environment"), 0, KEY_READ, &mut k), 0);
        check_code(RegCloseKey(k), 0);
        check_code(RegCloseKey(HKEY_LOCAL_MACHINE), 0);
        check_code(RegCloseKey(0x1234), ERROR_INVALID_HANDLE);
        // HKEY_CURRENT_USER is the token user's hive.
        check_code(RegOpenKeyExW(HKEY_CURRENT_USER, w!("Control Panel\\International"), 0, KEY_READ, &mut k), 0);
        let mut lsize = 64u32;
        check_code(
            RegGetValueW(
                k,
                core::ptr::null(),
                w!("LocaleName"),
                RRF_RT_REG_SZ,
                &mut kind,
                buf.as_mut_ptr().cast(),
                &mut lsize,
            ),
            0,
        );
        check(wide_eq(buf.as_ptr(), b"en-US"));
        check_code(RegCloseKey(k), 0);
    }
}

unsafe fn registry_write() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let (mut k, mut d): (HKey, u32) = (0, 0);
        let class = w!("PolyClass");
        check_code(
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                w!("Software\\PolyA32\\Deep\\Er"),
                0,
                class,
                0,
                KEY_ALL_ACCESS,
                core::ptr::null(),
                &mut k,
                &mut d,
            ),
            0,
        );
        check_got(d == REG_CREATED_NEW_KEY, d);
        let mut again: HKey = 0;
        check_code(
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                w!("Software\\PolyA32\\Deep\\Er"),
                0,
                core::ptr::null(),
                0,
                KEY_READ,
                core::ptr::null(),
                &mut again,
                &mut d,
            ),
            0,
        );
        check_got(d == REG_OPENED_EXISTING_KEY, d);
        check_code(RegCloseKey(again), 0);
        // Values of every common type.
        let dword = 0x1234_5678u32.to_le_bytes();
        let qword = 0x1122_3344_5566_7788u64.to_le_bytes();
        check_code(RegSetValueExW(k, w!("dw"), 0, REG_DWORD, dword.as_ptr(), 4), 0);
        check_code(RegSetValueExW(k, w!("qw"), 0, REG_QWORD, qword.as_ptr(), 8), 0);
        check_code(RegSetValueExW(k, w!("bin"), 0, REG_BINARY, [1u8, 2, 3].as_ptr(), 3), 0);
        let sz = w!("text");
        check_code(RegSetValueExW(k, w!("sz"), 0, REG_SZ, sz.cast(), 10), 0);
        let multi = [0x61u16, 0, 0x62, 0, 0];
        check_code(RegSetValueExW(k, w!("multi"), 0, REG_MULTI_SZ, multi.as_ptr().cast(), 10), 0);
        // A string stored without its NUL: RegGetValueW adds one.
        check_code(RegSetValueExW(k, w!("nonul"), 0, REG_SZ, w!("ab").cast(), 4), 0);
        check_code(SetEnvironmentVariableW(w!("POLYA32"), w!("val")) as u32, 1);
        let exp = w!("%POLYA32%\\x");
        check_code(RegSetValueExW(k, w!("exp"), 0, REG_EXPAND_SZ, exp.cast(), 24), 0);
        check_code(RegSetValueExA(k, a!("ansi"), 0, REG_SZ, c"caf\xE9".as_ptr().cast(), 5), 0);
        check_code(RegSetValueExW(k, core::ptr::null(), 0, REG_SZ, sz.cast(), 10), 0);
        check_code(RegSetValueExW(k, w!("bad"), 1, REG_SZ, sz.cast(), 10), ERROR_INVALID_PARAMETER);
        // RegGetValueW: types, sizes, termination, expansion.
        let (mut kind, mut size) = (0u32, 8u32);
        let mut buf = [0u8; 64];
        check_code(
            RegGetValueW(k, core::ptr::null(), w!("dw"), RRF_RT_DWORD, &mut kind, buf.as_mut_ptr(), &mut size),
            0,
        );
        check_got(kind == REG_DWORD && size == 4 && u32_at(&buf, 0) == 0x1234_5678, size);
        size = 8;
        check_code(
            RegGetValueW(k, core::ptr::null(), w!("qw"), RRF_RT_QWORD, &mut kind, buf.as_mut_ptr(), &mut size),
            0,
        );
        check_got(kind == REG_QWORD && size == 8, size);
        size = 8;
        check_code(
            RegGetValueW(k, core::ptr::null(), w!("bin"), RRF_RT_DWORD, &mut kind, buf.as_mut_ptr(), &mut size),
            1629,
        );
        size = 64;
        check_code(
            RegGetValueW(k, core::ptr::null(), w!("nonul"), RRF_RT_REG_SZ, &mut kind, buf.as_mut_ptr(), &mut size),
            0,
        );
        check_got(size == 6 && wide_eq(buf.as_ptr().cast(), b"ab"), size);
        size = 64;
        check_code(
            RegGetValueW(k, core::ptr::null(), w!("exp"), RRF_RT_REG_SZ, &mut kind, buf.as_mut_ptr(), &mut size),
            0,
        );
        check_got(kind == REG_SZ && wide_eq(buf.as_ptr().cast(), b"val\\x"), size);
        size = 64;
        check_code(
            RegGetValueW(
                k,
                core::ptr::null(),
                w!("exp"),
                RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND,
                &mut kind,
                buf.as_mut_ptr(),
                &mut size,
            ),
            0,
        );
        check_got(kind == REG_EXPAND_SZ && wide_eq(buf.as_ptr().cast(), b"%POLYA32%\\x"), size);
        size = 64;
        check_code(
            RegGetValueW(k, core::ptr::null(), w!("ansi"), RRF_RT_ANY, &mut kind, buf.as_mut_ptr(), &mut size),
            0,
        );
        check_got(size == 10 && u32_at(&buf, 4) == 0x00E9_0066, size);
        size = 4;
        buf[0] = 0x55;
        check_code(
            RegGetValueW(
                k,
                core::ptr::null(),
                w!("multi"),
                RRF_RT_ANY | RRF_ZEROONFAILURE,
                &mut kind,
                buf.as_mut_ptr(),
                &mut size,
            ),
            ERROR_MORE_DATA,
        );
        check_got(size == 10 && buf[0] == 0, size);
        // Enumerate the values: 9 of them, then no more.
        let mut n = 0;
        loop {
            let mut name = [0u16; 32];
            let (mut nlen, mut dsize) = (32u32, 64u32);
            let r = RegEnumValueW(
                k,
                n,
                name.as_mut_ptr(),
                &mut nlen,
                core::ptr::null_mut(),
                &mut kind,
                buf.as_mut_ptr(),
                &mut dsize,
            );
            if r == ERROR_NO_MORE_ITEMS {
                break;
            }
            check_code(r, 0);
            check_got(nlen as usize == wlen(name.as_ptr()), nlen);
            n += 1;
        }
        check_got(n == 9, n);
        // A name buffer too small (none holds even the NUL).
        let mut name = [0u16; 2];
        let mut nlen = 0u32;
        check_code(
            RegEnumValueW(
                k,
                0,
                name.as_mut_ptr(),
                &mut nlen,
                core::ptr::null_mut(),
                &mut kind,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            ),
            ERROR_MORE_DATA,
        );
        // RegQueryInfoKeyW.
        let (mut subkeys, mut values, mut max_name, mut max_data) = (9u32, 0u32, 0u32, 0u32);
        let mut cls = [0u16; 16];
        let mut cls_len = 16u32;
        check_code(
            RegQueryInfoKeyW(
                k,
                cls.as_mut_ptr(),
                &mut cls_len,
                core::ptr::null_mut(),
                &mut subkeys,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                &mut values,
                &mut max_name,
                &mut max_data,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            ),
            0,
        );
        check_got(subkeys == 0 && values == 9 && max_name == 5 && max_data == 24, max_data);
        check_got(cls_len == 9 && wide_eq(cls.as_ptr(), b"PolyClass"), cls_len);
        check_code(RegDeleteValueW(k, w!("bin")), 0);
        check_code(RegDeleteValueW(k, w!("bin")), ERROR_FILE_NOT_FOUND);
        check_code(RegCloseKey(k), 0);
        // Enumerate the parent's subkeys.
        let mut parent: HKey = 0;
        check_code(RegOpenKeyExW(HKEY_CURRENT_USER, w!("Software\\PolyA32\\Deep"), 0, KEY_READ, &mut parent), 0);
        let mut sub = [0u16; 16];
        let mut slen = 16u32;
        let mut time = 0u64;
        check_code(
            RegEnumKeyExW(
                parent,
                0,
                sub.as_mut_ptr(),
                &mut slen,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                &mut time,
            ),
            0,
        );
        check_got(slen == 2 && wide_eq(sub.as_ptr(), b"Er") && time != 0, slen);
        slen = 16;
        check_code(
            RegEnumKeyExW(
                parent,
                1,
                sub.as_mut_ptr(),
                &mut slen,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                &mut time,
            ),
            ERROR_NO_MORE_ITEMS,
        );
        check_code(RegCloseKey(parent), 0);
        // A key with subkeys cannot be deleted; leaves first.
        check_code(RegDeleteKeyW(HKEY_CURRENT_USER, w!("Software\\PolyA32\\Deep")), ERROR_ACCESS_DENIED);
        check_code(RegDeleteKeyW(HKEY_CURRENT_USER, w!("Software\\PolyA32\\Deep\\Er")), 0);
        check_code(RegDeleteKeyW(HKEY_CURRENT_USER, w!("Software\\PolyA32\\Deep")), 0);
        check_code(RegDeleteKeyW(HKEY_CURRENT_USER, w!("Software\\PolyA32")), 0);
        check_code(
            RegOpenKeyExW(HKEY_CURRENT_USER, w!("Software\\PolyA32"), 0, KEY_READ, &mut k),
            ERROR_FILE_NOT_FOUND,
        );
    }
}

/// `HKEY_CLASSES_ROOT`: a new key goes under `HKLM\Software\Classes`.
unsafe fn classes_root() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let (mut k, mut d): (HKey, u32) = (0, 0);
        check_code(
            RegCreateKeyExW(
                HKEY_CLASSES_ROOT,
                w!(".polya32"),
                0,
                core::ptr::null(),
                0,
                KEY_ALL_ACCESS,
                core::ptr::null(),
                &mut k,
                &mut d,
            ),
            0,
        );
        check_code(RegSetValueExW(k, core::ptr::null(), 0, REG_SZ, w!("PolyFile").cast(), 18), 0);
        check_code(RegCloseKey(k), 0);
        check_code(RegOpenKeyExW(HKEY_LOCAL_MACHINE, w!("SOFTWARE\\Classes\\.polya32"), 0, KEY_READ, &mut k), 0);
        check_code(RegCloseKey(k), 0);
        let mut buf = [0u16; 16];
        let (mut kind, mut size) = (0u32, 32u32);
        check_code(
            RegGetValueW(
                HKEY_CLASSES_ROOT,
                w!(".polya32"),
                core::ptr::null(),
                RRF_RT_REG_SZ,
                &mut kind,
                buf.as_mut_ptr().cast(),
                &mut size,
            ),
            0,
        );
        check(wide_eq(buf.as_ptr(), b"PolyFile"));
        check_code(RegDeleteKeyW(HKEY_CLASSES_ROOT, w!(".polya32")), 0);
    }
}

unsafe fn token() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let mut t: Handle = 0;
        check(OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut t) != 0);
        // The size first.
        let mut len = 0u32;
        check_fail(GetTokenInformation(t, TOKEN_USER, core::ptr::null_mut(), 0, &mut len), ERROR_INSUFFICIENT_BUFFER);
        check_got(len == 44, len);
        let mut buf = [0u64; 64];
        let p = buf.as_mut_ptr().cast::<u8>();
        check(GetTokenInformation(t, TOKEN_USER, p, 512, &mut len) != 0);
        let user = *(p as *const *const u8);
        check(IsValidSid(user) != 0 && user == p.add(16));
        let mut s: *mut u16 = core::ptr::null_mut();
        check(ConvertSidToStringSidW(user, &mut s) != 0);
        check(wide_eq(s, b"S-1-5-21-0-0-0-1000"));
        check(LocalFree(s.cast()).is_null());
        // Elevated, high integrity, the default elevation type.
        let mut v = 0u32;
        check(GetTokenInformation(t, TOKEN_ELEVATION, (&raw mut v).cast(), 4, &mut len) != 0);
        check_got(v == 1, v);
        check(GetTokenInformation(t, TOKEN_ELEVATION_TYPE, (&raw mut v).cast(), 4, &mut len) != 0);
        check_got(v == 1, v);
        check(GetTokenInformation(t, TOKEN_INTEGRITY_LEVEL, p, 512, &mut len) != 0);
        let label = *(p as *const *const u8);
        check_got(
            *GetSidSubAuthorityCount(label) == 1 && *GetSidSubAuthority(label, 0) == 0x3000,
            *GetSidSubAuthority(label, 0),
        );
        // Administrators is among the groups, and CheckTokenMembership agrees.
        let mut admins: *mut u8 = core::ptr::null_mut();
        check(AllocateAndInitializeSid(&SECURITY_NT_AUTHORITY, 2, 32, 544, 0, 0, 0, 0, 0, 0, &mut admins) != 0);
        check(GetTokenInformation(t, TOKEN_GROUPS, p, 512, &mut len) != 0);
        let count = *(p as *const u32);
        let mut found = false;
        for i in 0..count as usize {
            let g = *(p.add(8 + 16 * i) as *const *const u8);
            found |= EqualSid(g, admins) != 0;
        }
        check_got(found, count);
        let mut member = 0;
        check(CheckTokenMembership(0, admins, &mut member) != 0);
        check_got(member == 1, member as u32);
        let mut nobody: *mut u8 = core::ptr::null_mut();
        check(AllocateAndInitializeSid(&SECURITY_NT_AUTHORITY, 2, 32, 546, 0, 0, 0, 0, 0, 0, &mut nobody) != 0);
        check(CheckTokenMembership(0, nobody, &mut member) != 0);
        check_got(member == 0, member as u32);
        check(FreeSid(nobody).is_null());
        check_fail(CheckTokenMembership(t, admins, &mut member), 1309);
        check(FreeSid(admins).is_null());
        check(CloseHandle(t) != 0);
        // Threads do not impersonate.
        check_fail(OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut t), ERROR_NO_TOKEN);
        // No query right.
        check(OpenProcessToken(GetCurrentProcess(), 0x80, &mut t) != 0);
        check_fail(GetTokenInformation(t, TOKEN_ELEVATION, (&raw mut v).cast(), 4, &mut len), ERROR_ACCESS_DENIED);
        check(CloseHandle(t) != 0);
        // GetUserNameW.
        let mut name = [0u16; 16];
        let mut size = 2u32;
        check_fail(GetUserNameW(name.as_mut_ptr(), &mut size), ERROR_INSUFFICIENT_BUFFER);
        check_got(size == 5, size);
        size = 16;
        check(GetUserNameW(name.as_mut_ptr(), &mut size) != 0);
        check_got(size == 5 && wide_eq(name.as_ptr(), b"poly"), size);
        let mut luid = 0u64;
        // A success leaves the last error 0 (ADR 0114).
        SetLastError(5);
        check(LookupPrivilegeValueW(core::ptr::null(), w!("SeDebugPrivilege"), &mut luid) != 0);
        check_got(luid == 20, luid as u32);
        check_code(GetLastError(), 0);
        adjust();
    }
}

/// `AdjustTokenPrivileges` (ADR 0114): enable a privilege the token holds,
/// see it enabled, get it back with `PreviousState`; one it lacks is
/// `ERROR_NOT_ALL_ASSIGNED`.
unsafe fn adjust() {
    const TOKEN_ADJUST_PRIVILEGES: u32 = 0x20;
    const TOKEN_PRIVILEGES: u32 = 3;
    // SAFETY: valid arguments throughout.
    unsafe {
        // TOKEN_PRIVILEGES with one entry: count, LUID (low, high), attributes.
        let tp = |luid: u32, attrs: u32| [1u32, luid, 0, attrs];
        let enabled = |t: Handle, luid: u32| -> Option<u32> {
            let mut buf = [0u32; 256];
            let mut len = 0;
            check(GetTokenInformation(t, TOKEN_PRIVILEGES, buf.as_mut_ptr().cast(), 1024, &mut len) != 0);
            (0..buf[0] as usize).find(|&i| buf[1 + 3 * i] == luid).map(|i| buf[3 + 3 * i])
        };
        let mut t: Handle = 0;
        check(OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut t) != 0);
        // SeRestorePrivilege (18): held, disabled.
        check_got(enabled(t, 18) == Some(0), enabled(t, 18).unwrap_or(99));
        let on = tp(18, 2);
        SetLastError(5);
        check(AdjustTokenPrivileges(t, 0, on.as_ptr().cast(), 0, core::ptr::null_mut(), core::ptr::null_mut()) != 0);
        check_code(GetLastError(), 0);
        check_got(enabled(t, 18) == Some(2), enabled(t, 18).unwrap_or(99));
        // Disable it again, asking for the previous state: just it, enabled.
        let off = tp(18, 0);
        let mut prev = [0u32; 8];
        let mut len = 0u32;
        check(AdjustTokenPrivileges(t, 0, off.as_ptr().cast(), 32, prev.as_mut_ptr().cast(), &mut len) != 0);
        check_got(len == 16 && prev[0] == 1 && prev[1] == 18 && prev[3] == 2, len);
        check_got(enabled(t, 18) == Some(0), enabled(t, 18).unwrap_or(99));
        // SeLockMemoryPrivilege (4) is not held: success, not all assigned.
        let lock = tp(4, 2);
        check(AdjustTokenPrivileges(t, 0, lock.as_ptr().cast(), 0, core::ptr::null_mut(), core::ptr::null_mut()) != 0);
        check_code(GetLastError(), 1300);
        check(CloseHandle(t) != 0);
        // Without TOKEN_ADJUST_PRIVILEGES.
        check(OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut t) != 0);
        let r = AdjustTokenPrivileges(t, 0, on.as_ptr().cast(), 0, core::ptr::null_mut(), core::ptr::null_mut());
        check_fail(r, ERROR_ACCESS_DENIED);
        check(CloseHandle(t) != 0);
    }
}

unsafe fn sids() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let mut everyone: *mut u8 = core::ptr::null_mut();
        check_fail(
            AllocateAndInitializeSid(&SECURITY_WORLD_SID_AUTHORITY, 9, 0, 0, 0, 0, 0, 0, 0, 0, &mut everyone),
            ERROR_INVALID_PARAMETER,
        );
        check(AllocateAndInitializeSid(&SECURITY_WORLD_SID_AUTHORITY, 1, 0, 0, 0, 0, 0, 0, 0, 0, &mut everyone) != 0);
        check_got(GetLengthSid(everyone) == 12, GetLengthSid(everyone));
        let mut copy = [0u8; 16];
        check_fail(CopySid(8, copy.as_mut_ptr(), everyone), ERROR_INSUFFICIENT_BUFFER);
        check(CopySid(16, copy.as_mut_ptr(), everyone) != 0);
        check(EqualSid(copy.as_ptr(), everyone) != 0);
        let mut from: *mut u8 = core::ptr::null_mut();
        check(ConvertStringSidToSidW(w!("WD"), &mut from) != 0);
        check(EqualSid(from, everyone) != 0);
        LocalFree(from);
        check(ConvertStringSidToSidW(w!("S-1-5-32-544"), &mut from) != 0);
        check(EqualSid(from, everyone) == 0);
        let mut s: *mut u16 = core::ptr::null_mut();
        check(ConvertSidToStringSidW(from, &mut s) != 0);
        check(wide_eq(s, b"S-1-5-32-544"));
        LocalFree(s.cast());
        LocalFree(from);
        check_fail(ConvertStringSidToSidW(w!("S-1-x"), &mut from), ERROR_INVALID_SID);
        let bad = [2u8, 0, 0, 0, 0, 0, 0, 0];
        check(IsValidSid(bad.as_ptr()) == 0);
        check_fail(ConvertSidToStringSidW(bad.as_ptr(), &mut s), ERROR_INVALID_SID);
        check(FreeSid(everyone).is_null());
    }
}

unsafe fn descriptors() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let mut admins: *mut u8 = core::ptr::null_mut();
        check(AllocateAndInitializeSid(&SECURITY_NT_AUTHORITY, 2, 32, 544, 0, 0, 0, 0, 0, 0, &mut admins) != 0);
        // An ACL with room for exactly two ACEs (Learn's size rule).
        let mut acl = [0u32; 32];
        let ap = acl.as_mut_ptr().cast::<u8>();
        let size = 8 + 2 * (8 + 16);
        check(InitializeAcl(ap, size, ACL_REVISION) != 0);
        check(AddAccessAllowedAce(ap, ACL_REVISION, GENERIC_ALL, admins) != 0);
        check(AddAccessDeniedAce(ap, ACL_REVISION, 0x1_0000, admins) != 0);
        check_fail(AddAccessAllowedAce(ap, ACL_REVISION, 1, admins), ERROR_ALLOTTED_SPACE_EXCEEDED);
        check(IsValidAcl(ap) != 0);
        let mut info = [0u8; 12];
        check(GetAclInformation(ap, info.as_mut_ptr(), 12, ACL_SIZE_INFORMATION) != 0);
        check_got(u32_at(&info, 0) == 2 && u32_at(&info, 4) == size && u32_at(&info, 8) == 0, u32_at(&info, 4));
        check_fail(GetAclInformation(ap, info.as_mut_ptr(), 8, ACL_SIZE_INFORMATION), ERROR_INSUFFICIENT_BUFFER);
        let mut ace: *mut u8 = core::ptr::null_mut();
        check(GetAce(ap, 1, &mut ace) != 0);
        check_got(*ace == 1 && ace == ap.add(32), u32::from(*ace));
        check_fail(GetAce(ap, 2, &mut ace), ERROR_INVALID_PARAMETER);
        // An absolute descriptor with that DACL and an owner.
        let mut sd = [0u64; 5];
        let sp = sd.as_mut_ptr().cast::<u8>();
        check_fail(InitializeSecurityDescriptor(sp, 2), ERROR_UNKNOWN_REVISION);
        check(InitializeSecurityDescriptor(sp, 1) != 0);
        check(SetSecurityDescriptorDacl(sp, 1, ap, 0) != 0);
        check(SetSecurityDescriptorOwner(sp, admins, 0) != 0);
        let (mut present, mut defaulted) = (0, 1);
        let mut got: *mut u8 = core::ptr::null_mut();
        check(GetSecurityDescriptorDacl(sp, &mut present, &mut got, &mut defaulted) != 0);
        check(present == 1 && got == ap && defaulted == 0);
        check(IsValidSecurityDescriptor(sp) != 0);
        check_got(GetSecurityDescriptorLength(sp) == 40 + 16 + size, GetSecurityDescriptorLength(sp));
        // Self-relative: the size first, then the copy.
        let mut rel = [0u64; 16];
        let rp = rel.as_mut_ptr().cast::<u8>();
        let mut rlen = 0u32;
        check_fail(MakeSelfRelativeSD(sp, rp, &mut rlen), ERROR_INSUFFICIENT_BUFFER);
        check_got(rlen == 20 + 16 + size, rlen);
        check(MakeSelfRelativeSD(sp, rp, &mut rlen) != 0);
        check(IsValidSecurityDescriptor(rp) != 0);
        check_got(GetSecurityDescriptorLength(rp) == rlen, GetSecurityDescriptorLength(rp));
        check(GetSecurityDescriptorDacl(rp, &mut present, &mut got, &mut defaulted) != 0);
        check(present == 1 && got > rp && got < rp.add(rlen as usize) && IsValidAcl(got) != 0);
        check(GetSecurityDescriptorOwner(rp, &mut got, &mut defaulted) != 0);
        check(EqualSid(got, admins) != 0);
        check_fail(SetSecurityDescriptorDacl(rp, 1, ap, 0), ERROR_INVALID_SECURITY_DESCR);
        // No DACL: present is FALSE.
        check(SetSecurityDescriptorDacl(sp, 0, core::ptr::null_mut(), 0) != 0);
        check(GetSecurityDescriptorDacl(sp, &mut present, &mut got, &mut defaulted) != 0);
        check(present == 0);
        check(DeleteAce(ap, 0) != 0);
        check(GetAclInformation(ap, info.as_mut_ptr(), 12, ACL_SIZE_INFORMATION) != 0);
        check_got(u32_at(&info, 0) == 1 && u32_at(&info, 8) == 24, u32_at(&info, 8));
        check(FreeSid(admins).is_null());
        SetLastError(0);
    }
}
