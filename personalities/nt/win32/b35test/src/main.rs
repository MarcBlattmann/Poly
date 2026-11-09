//! `b35test.exe` (ADR 0155): the program the kernel's ring-3 test
//! `nt_chrome_b35_program_passes` runs. It imports from `advapi32`,
//! `iphlpapi`, `netapi32`, `secur32`, `dhcpcsvc`, `dbghelp` and `kernel32`
//! and checks what Learn documents for the calls Chrome makes: ETW with no
//! listener, registry change notification, the adapter and interface
//! queries, the user and join information, the DHCP client and symbol
//! handler stubs.
//!
//! It exits through `ExitProcess(PASS)` when every check passed; a failed
//! check prints `b35test: step N failed (got G)` and exits with `0x80 + N`.

#![no_std]
#![no_main]
#![allow(clippy::missing_safety_doc)]

use core::sync::atomic::{AtomicU32, Ordering};
use poly_win32_rt as _;

type Handle = isize;
const INVALID_HANDLE_VALUE: Handle = -1;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ExitProcess(code: u32) -> !;
    fn GetStdHandle(which: u32) -> Handle;
    fn WriteFile(h: Handle, buf: *const u8, len: u32, written: *mut u32, ov: *mut u8) -> i32;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn CreateEventW(sa: *const u8, manual: i32, initial: i32, name: *const u16) -> Handle;
    fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    fn CloseHandle(h: Handle) -> i32;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn EventRegister(provider: *const u8, cb: usize, ctx: usize, h: *mut u64) -> u32;
    fn EventUnregister(h: u64) -> u32;
    fn EventSetInformation(h: u64, class: u32, info: *const u8, len: u32) -> u32;
    fn EventEnabled(h: u64, desc: *const u8) -> u8;
    fn EventProviderEnabled(h: u64, level: u8, keyword: u64) -> u8;
    fn EventWrite(h: u64, desc: *const u8, count: u32, data: *const u8) -> u32;
    fn EventWriteString(h: u64, level: u8, keyword: u64, s: *const u16) -> u32;
    fn EventActivityIdControl(code: u32, id: *mut u8) -> u32;
    fn RegisterTraceGuidsW(
        cb: usize,
        ctx: usize,
        guid: *const u8,
        n: u32,
        reg: *const u8,
        a: *const u16,
        b: *const u16,
        h: *mut u64,
    ) -> u32;
    fn UnregisterTraceGuids(h: u64) -> u32;
    fn TraceEvent(h: u64, header: *const u8) -> u32;
    fn GetTraceEnableFlags(h: u64) -> u32;
    fn GetTraceEnableLevel(h: u64) -> u8;
    fn GetTraceLoggerHandle(buf: *const u8) -> u64;
    fn RegCreateKeyExW(
        key: isize,
        sub: *const u16,
        reserved: u32,
        class: *const u16,
        options: u32,
        sam: u32,
        sa: *const u8,
        out: *mut isize,
        disposition: *mut u32,
    ) -> u32;
    fn RegSetValueExW(key: isize, name: *const u16, reserved: u32, kind: u32, data: *const u8, len: u32) -> u32;
    fn RegNotifyChangeKeyValue(key: isize, subtree: i32, filter: u32, event: Handle, asynchronous: i32) -> u32;
    fn RegCloseKey(key: isize) -> u32;
    fn RegDeleteKeyExW(key: isize, sub: *const u16, sam: u32, reserved: u32) -> u32;
}

#[link(name = "iphlpapi", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetAdaptersAddresses(family: u32, flags: u32, reserved: *mut u8, addrs: *mut u8, size: *mut u32) -> u32;
    fn GetAdaptersInfo(info: *mut u8, size: *mut u32) -> u32;
    fn GetIfTable2(table: *mut *mut u8) -> u32;
    fn FreeMibTable(table: *mut u8);
    fn GetBestInterface(dest: u32, index: *mut u32) -> u32;
    fn NotifyAddrChange(handle: *mut Handle, ov: *mut u8) -> u32;
    fn CancelIPChangeNotify(ov: *mut u8) -> i32;
}

#[link(name = "netapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn NetUserGetInfo(server: *const u16, user: *const u16, level: u32, buf: *mut *mut u8) -> u32;
    fn NetApiBufferFree(buf: *mut u8) -> u32;
    fn NetGetJoinInformation(server: *const u16, name: *mut *mut u16, kind: *mut u32) -> u32;
    fn DsRoleGetPrimaryDomainInformation(server: *const u16, level: u32, buf: *mut *mut u8) -> u32;
    fn DsRoleFreeMemory(buf: *mut u8);
}

#[link(name = "secur32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetUserNameExW(format: u32, buf: *mut u16, size: *mut u32) -> u8;
    fn AcquireCredentialsHandleW(
        principal: *const u16,
        package: *const u16,
        usage: u32,
        logon: *const u8,
        auth: *const u8,
        key_fn: usize,
        key_arg: *const u8,
        cred: *mut u8,
        expiry: *mut u8,
    ) -> i32;
}

#[link(name = "crypt32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CertOpenStore(provider: *const u8, encoding: u32, prov: usize, flags: u32, para: *const u8) -> usize;
    fn CertCloseStore(store: usize, flags: u32) -> i32;
    fn CertAddStoreToCollection(collection: usize, sibling: usize, flags: u32, priority: i32) -> i32;
    fn CertControlStore(store: usize, flags: u32, ctrl: u32, para: *mut isize) -> i32;
    fn CertEnumCertificatesInStore(store: usize, prev: usize) -> usize;
    fn CryptProtectMemory(data: *mut u8, len: u32, flags: u32) -> i32;
    fn CryptUnprotectMemory(data: *mut u8, len: u32, flags: u32) -> i32;
}

#[link(name = "dhcpcsvc", kind = "raw-dylib")]
unsafe extern "system" {
    fn DhcpCApiInitialize(version: *mut u32) -> u32;
    fn DhcpRequestParams(
        flags: u32,
        reserved: *const u8,
        adapter: *const u16,
        class: *const u8,
        send: *const u8,
        recd: *const u8,
        buf: *mut u8,
        size: *mut u32,
        request: *const u16,
    ) -> u32;
}

#[link(name = "dbghelp", kind = "raw-dylib")]
unsafe extern "system" {
    fn SymInitialize(process: isize, path: *const u8, invade: i32) -> i32;
    fn SymSetOptions(options: u32) -> u32;
    fn SymFromAddr(process: isize, address: u64, disp: *mut u64, symbol: *mut u8) -> i32;
    fn SymCleanup(process: isize) -> i32;
    fn MiniDumpWriteDump(
        process: isize,
        pid: u32,
        file: isize,
        kind: u32,
        e: *const u8,
        u: *const u8,
        c: *const u8,
    ) -> i32;
}

const PASS: u32 = 0x57;
const WAIT_TIMEOUT: u32 = 258;
const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
const KEY_ALL_ACCESS: u32 = 0xF003F;
const REG_DWORD: u32 = 4;
const REG_NOTIFY_CHANGE_LAST_SET: u32 = 4;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_BUFFER_OVERFLOW: u32 = 111;
const ERROR_MORE_DATA: u32 = 234;
const ERROR_NONE_MAPPED: u32 = 1332;
const ERROR_IO_PENDING: u32 = 997;
const ERROR_NOT_FOUND: u32 = 1168;
const ERROR_MOD_NOT_FOUND: u32 = 126;
const ERROR_NOT_SUPPORTED: u32 = 50;
const NERR_USER_NOT_FOUND: u32 = 2221;
const SEC_E_SECPKG_NOT_FOUND: i32 = 0x8009_0305_u32 as i32;

// ----- reporting

fn out(bytes: &[u8]) {
    // SAFETY: kernel32 calls with valid buffers.
    unsafe {
        let h = GetStdHandle(0xFFFF_FFF5);
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

fn check_got(ok: bool, got: u32) {
    let step = STEP.fetch_add(1, Ordering::Relaxed) + 1;
    if !ok {
        out(b"b35test: step ");
        out_num(step);
        out(b" failed (got ");
        out_num(got);
        out(b")\n");
        // SAFETY: never returns.
        unsafe { ExitProcess(0x80 + (step & 0x7F)) }
    }
}

fn check(ok: bool) {
    // SAFETY: a plain kernel32 call.
    check_got(ok, unsafe { GetLastError() });
}

/// A call returning a Win32 error code: it must be `want`.
fn is(got: u32, want: u32) {
    check_got(got == want, got);
}

fn w(s: &str, buf: &mut [u16; 64]) -> *const u16 {
    for (i, c) in s.encode_utf16().chain([0]).enumerate() {
        buf[i] = c;
    }
    buf.as_ptr()
}

unsafe fn wide_eq(p: *const u16, want: &str) -> bool {
    // SAFETY: a NUL-terminated string.
    unsafe {
        let mut i = 0;
        for c in want.encode_utf16() {
            if *p.add(i) != c {
                return false;
            }
            i += 1;
        }
        *p.add(i) == 0
    }
}

fn rd32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn rd64(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

// ----- ETW

unsafe fn etw() {
    let guid = [0x42u8; 16];
    let desc = [0u8; 16];
    // SAFETY: advapi32 calls with valid buffers.
    unsafe {
        let mut h = 0u64;
        is(EventRegister(core::ptr::null(), 0, 0, &mut h), ERROR_INVALID_PARAMETER);
        is(EventRegister(guid.as_ptr(), 0, 0, core::ptr::null_mut()), ERROR_INVALID_PARAMETER);
        is(EventRegister(guid.as_ptr(), 0, 0, &mut h), 0);
        check(h != 0 && h != u64::MAX);
        let mut h2 = 0u64;
        is(EventRegister(guid.as_ptr(), 0, 0, &mut h2), 0);
        check(h2 != h);
        // Nobody listens.
        check(EventEnabled(h, desc.as_ptr()) == 0);
        check(EventProviderEnabled(h, 5, u64::MAX) == 0);
        is(EventWrite(h, desc.as_ptr(), 0, core::ptr::null()), 0);
        let mut data = [0u8; 16];
        is(EventWrite(h, desc.as_ptr(), 1, data.as_mut_ptr()), 0);
        is(EventWrite(h, desc.as_ptr(), 129, data.as_ptr()), ERROR_INVALID_PARAMETER);
        is(EventWrite(h + 999, desc.as_ptr(), 0, core::ptr::null()), ERROR_INVALID_HANDLE);
        let mut s = [0u16; 64];
        is(EventWriteString(h, 4, 0, w("hello", &mut s)), 0);
        is(EventSetInformation(h, 2, data.as_ptr(), 8), 0);
        is(EventSetInformation(h, 99, data.as_ptr(), 8), ERROR_INVALID_PARAMETER);
        is(EventSetInformation(h + 999, 2, data.as_ptr(), 8), ERROR_INVALID_HANDLE);
        // Activity ids.
        let mut id = [0xAAu8; 16];
        is(EventActivityIdControl(1, id.as_mut_ptr()), 0);
        check(id == [0; 16]);
        let mut made = [0u8; 16];
        is(EventActivityIdControl(3, made.as_mut_ptr()), 0);
        check(made != [0; 16] && made[7] >> 4 == 4);
        is(EventActivityIdControl(2, made.as_mut_ptr()), 0);
        let mut got = [0u8; 16];
        is(EventActivityIdControl(1, got.as_mut_ptr()), 0);
        check(got == made);
        is(EventActivityIdControl(77, got.as_mut_ptr()), ERROR_INVALID_PARAMETER);
        is(EventUnregister(h), 0);
        is(EventUnregister(h), ERROR_INVALID_HANDLE);
        is(EventUnregister(h2), 0);

        // The classic API.
        let mut t = 0u64;
        is(
            RegisterTraceGuidsW(
                0x1000,
                0,
                guid.as_ptr(),
                0,
                core::ptr::null(),
                core::ptr::null(),
                core::ptr::null(),
                &mut t,
            ),
            0,
        );
        check(t != 0);
        is(
            RegisterTraceGuidsW(
                0,
                0,
                guid.as_ptr(),
                0,
                core::ptr::null(),
                core::ptr::null(),
                core::ptr::null(),
                &mut t,
            ),
            ERROR_INVALID_PARAMETER,
        );
        check(GetTraceEnableFlags(t) == 0 && GetTraceEnableLevel(t) == 0);
        let mut header = [0u8; 64];
        header[0] = 48;
        is(TraceEvent(t, header.as_ptr()), 0);
        header[0] = 8;
        is(TraceEvent(t, header.as_ptr()), ERROR_INVALID_PARAMETER);
        is(TraceEvent(t, core::ptr::null()), ERROR_INVALID_PARAMETER);
        let mut wnode = [0u8; 48];
        wnode[8..16].copy_from_slice(&0x77u64.to_le_bytes());
        check(GetTraceLoggerHandle(wnode.as_ptr()) == 0x77);
        check(GetTraceLoggerHandle(core::ptr::null()) == u64::MAX);
        is(UnregisterTraceGuids(t), 0);
        is(UnregisterTraceGuids(t), ERROR_INVALID_HANDLE);
    }
}

// ----- RegNotifyChangeKeyValue

unsafe fn reg_notify() {
    let mut buf = [0u16; 64];
    // SAFETY: advapi32/kernel32 calls with valid buffers.
    unsafe {
        let mut key: isize = 0;
        let mut disp = 0u32;
        is(
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                w("Software\\PolyB35Test", &mut buf),
                0,
                core::ptr::null(),
                0,
                KEY_ALL_ACCESS,
                core::ptr::null(),
                &mut key,
                &mut disp,
            ),
            0,
        );
        let ev = CreateEventW(core::ptr::null(), 1, 0, core::ptr::null());
        check(ev != 0);
        // Argument errors.
        is(RegNotifyChangeKeyValue(key, 0, 0, ev, 1), ERROR_INVALID_PARAMETER);
        is(RegNotifyChangeKeyValue(key, 0, 0x4000_0000, ev, 1), ERROR_INVALID_PARAMETER);
        is(RegNotifyChangeKeyValue(key, 0, REG_NOTIFY_CHANGE_LAST_SET, 0, 1), ERROR_INVALID_PARAMETER);
        // Asynchronous: nothing yet, then a change sets the event.
        is(RegNotifyChangeKeyValue(key, 0, REG_NOTIFY_CHANGE_LAST_SET, ev, 1), 0);
        check_got(WaitForSingleObject(ev, 0) == WAIT_TIMEOUT, 1);
        let v = 5u32.to_le_bytes();
        is(RegSetValueExW(key, w("v", &mut buf), 0, REG_DWORD, v.as_ptr(), 4), 0);
        check_got(WaitForSingleObject(ev, 2000) == 0, 2);
        // Re-armed, and the next change signals again.
        is(RegNotifyChangeKeyValue(key, 1, REG_NOTIFY_CHANGE_LAST_SET | 0x1000_0000, ev, 1), 0);
        let v2 = 6u32.to_le_bytes();
        is(RegSetValueExW(key, w("v", &mut buf), 0, REG_DWORD, v2.as_ptr(), 4), 0);
        check_got(WaitForSingleObject(ev, 2000) == 0, 3);
        check(CloseHandle(ev) != 0);
        is(RegCloseKey(key), 0);
        is(RegDeleteKeyExW(HKEY_CURRENT_USER, w("Software\\PolyB35Test", &mut buf), 0, 0), 0);
    }
}

// ----- iphlpapi

unsafe fn iphlp() {
    // 8-byte aligned buffer for the adapter list.
    #[repr(align(8))]
    struct Buf([u8; 16384]);
    let mut buf = Buf([0; 16384]);
    // SAFETY: iphlpapi/kernel32 calls with valid buffers.
    unsafe {
        let mut size = 0u32;
        is(GetAdaptersAddresses(0, 0, core::ptr::null_mut(), core::ptr::null_mut(), &mut size), ERROR_BUFFER_OVERFLOW);
        check_got(size >= 448, size);
        size = buf.0.len() as u32;
        is(GetAdaptersAddresses(0, 0x80, core::ptr::null_mut(), buf.0.as_mut_ptr(), &mut size), 0);
        let base = buf.0.as_ptr() as u64;
        // Walk the chain: the loopback is there with 127.0.0.1; every adapter has a name and a unicast address.
        let mut at = 0usize;
        let (mut loopback, mut adapters) = (false, 0);
        loop {
            check(rd32(&buf.0, at) == 448);
            adapters += 1;
            let if_type = rd32(&buf.0, at + 100);
            let name = rd64(&buf.0, at + 16);
            check(name >= base);
            let uni = rd64(&buf.0, at + 24);
            check(uni >= base);
            let sa = (rd64(&buf.0, (uni - base) as usize + 16) - base) as usize;
            if if_type == 24 {
                loopback = true;
                check(buf.0[sa..sa + 2] == [2, 0] && buf.0[sa + 4..sa + 8] == [127, 0, 0, 1]);
                check(buf.0[(uni - base) as usize + 56] == 8);
            }
            let next = rd64(&buf.0, at + 8);
            if next == 0 {
                break;
            }
            at = (next - base) as usize;
        }
        check(loopback && adapters >= 1);
        // A short buffer is reported with the size, a bad family is refused.
        let mut small = 10u32;
        is(GetAdaptersAddresses(2, 0, core::ptr::null_mut(), buf.0.as_mut_ptr(), &mut small), ERROR_BUFFER_OVERFLOW);
        check(small >= 448);
        is(GetAdaptersAddresses(99, 0, core::ptr::null_mut(), buf.0.as_mut_ptr(), &mut size), ERROR_INVALID_PARAMETER);

        // GetAdaptersInfo lists NICs only (none, or buffer-sized answers, are fine).
        let mut isz = buf.0.len() as u32;
        let r = GetAdaptersInfo(buf.0.as_mut_ptr(), &mut isz);
        check_got(r == 0 || r == 232 || r == ERROR_BUFFER_OVERFLOW, r);

        // GetIfTable2 / FreeMibTable.
        let mut table: *mut u8 = core::ptr::null_mut();
        is(GetIfTable2(&mut table), 0);
        check(!table.is_null());
        let rows = rd32(core::slice::from_raw_parts(table, 8), 0) as usize;
        check(rows >= 1);
        let t = core::slice::from_raw_parts(table, 8 + rows * 1352);
        check((0..rows).any(|i| rd32(t, 8 + i * 1352 + 1128) == 24));
        FreeMibTable(table);

        // The best interface to 127.0.0.1 is the loopback (index 1).
        let mut idx = 0u32;
        is(GetBestInterface(u32::from_le_bytes([127, 0, 0, 1]), &mut idx), 0);
        check_got(idx == 1, idx);

        // Address-change notification: pending until cancelled.
        let mut ov = [0u64; 4];
        let ev = CreateEventW(core::ptr::null(), 1, 0, core::ptr::null());
        ov[3] = ev as u64;
        let mut hh: Handle = 0;
        is(NotifyAddrChange(&mut hh, ov.as_mut_ptr().cast()), ERROR_IO_PENDING);
        check_got(WaitForSingleObject(ev, 0) == WAIT_TIMEOUT, 1);
        check(CancelIPChangeNotify(ov.as_mut_ptr().cast()) == 1);
        check_got(WaitForSingleObject(ev, 0) == 0, 2);
        check(CancelIPChangeNotify(ov.as_mut_ptr().cast()) == 0);
        is(GetLastError(), ERROR_NOT_FOUND);
        CloseHandle(ev);
    }
}

// ----- netapi32 and secur32

unsafe fn identity() {
    let mut s = [0u16; 64];
    // SAFETY: netapi32/secur32 calls with valid buffers.
    unsafe {
        let mut p: *mut u8 = core::ptr::null_mut();
        is(NetUserGetInfo(core::ptr::null(), w("poly", &mut s), 1, &mut p), 0);
        let info = core::slice::from_raw_parts(p, 56);
        let name = rd64(info, 0) as *const u16;
        check(wide_eq(name, "poly"));
        check(rd32(info, 20) == 2 && rd64(info, 8) == 0);
        is(NetApiBufferFree(p), 0);
        is(NetApiBufferFree(core::ptr::null_mut()), 0);
        is(NetUserGetInfo(core::ptr::null(), w("nobody", &mut s), 1, &mut p), NERR_USER_NOT_FOUND);
        is(NetUserGetInfo(core::ptr::null(), w("poly", &mut s), 77, &mut p), 124);

        let mut jn: *mut u16 = core::ptr::null_mut();
        let mut kind = 0u32;
        is(NetGetJoinInformation(core::ptr::null(), &mut jn, &mut kind), 0);
        check(kind == 2 && wide_eq(jn, "WORKGROUP"));
        NetApiBufferFree(jn.cast());
        let mut ds: *mut u8 = core::ptr::null_mut();
        is(DsRoleGetPrimaryDomainInformation(core::ptr::null(), 1, &mut ds), 0);
        check(rd32(core::slice::from_raw_parts(ds, 48), 0) == 0);
        DsRoleFreeMemory(ds);

        // GetUserNameExW: the SAM-compatible name, the buffer rule, formats without a directory.
        let mut name = [0u16; 64];
        let mut n = 64u32;
        check(GetUserNameExW(2, name.as_mut_ptr(), &mut n) == 1);
        check(n > 5 && name[n as usize] == 0 && name[(n - 5) as usize..n as usize] == [92, 112, 111, 108, 121]);
        let want = n;
        let mut short = 3u32;
        check(GetUserNameExW(2, name.as_mut_ptr(), &mut short) == 0);
        is(GetLastError(), ERROR_MORE_DATA);
        check_got(short == want + 1, short);
        n = 64;
        check(GetUserNameExW(8, name.as_mut_ptr(), &mut n) == 0);
        is(GetLastError(), ERROR_NONE_MAPPED);
        n = 64;
        check(GetUserNameExW(0, name.as_mut_ptr(), &mut n) == 0);
        is(GetLastError(), ERROR_INVALID_PARAMETER);

        // SSPI: no security packages.
        let mut cred = [0u8; 16];
        let mut exp = [0u8; 8];
        let r = AcquireCredentialsHandleW(
            core::ptr::null(),
            w("Negotiate", &mut s),
            2,
            core::ptr::null(),
            core::ptr::null(),
            0,
            core::ptr::null(),
            cred.as_mut_ptr(),
            exp.as_mut_ptr(),
        );
        check_got(r == SEC_E_SECPKG_NOT_FOUND, r as u32);
    }
}

// ----- dhcpcsvc and dbghelp

unsafe fn dhcp_and_symbols() {
    // SAFETY: dhcpcsvc/dbghelp calls with valid buffers.
    unsafe {
        let mut v = 0u32;
        is(DhcpCApiInitialize(&mut v), 0);
        check(v == 2);
        // One requested option (252, WPAD) comes back empty.
        let mut param = [0u8; 32];
        param[4..8].copy_from_slice(&252u32.to_le_bytes());
        param[16..24].copy_from_slice(&0xDEADu64.to_le_bytes());
        param[24..28].copy_from_slice(&9u32.to_le_bytes());
        let mut array = [0u8; 16];
        array[4..8].copy_from_slice(&1u32.to_le_bytes());
        array[8..16].copy_from_slice(&(param.as_mut_ptr() as u64).to_le_bytes());
        let mut buf = [0u8; 64];
        let mut size = 64u32;
        let mut s = [0u16; 64];
        is(
            DhcpRequestParams(
                1,
                core::ptr::null(),
                w("{ADAPTER}", &mut s),
                core::ptr::null(),
                core::ptr::null(),
                array.as_ptr(),
                buf.as_mut_ptr(),
                &mut size,
                core::ptr::null(),
            ),
            0,
        );
        check(rd64(&param, 16) == 0 && rd32(&param, 24) == 0);

        check(SymInitialize(-1, core::ptr::null(), 0) == 1);
        check(SymSetOptions(0x1) == 0x1);
        let mut disp = 0u64;
        let mut sym = [0u8; 128];
        check(SymFromAddr(-1, 0x1000, &mut disp, sym.as_mut_ptr()) == 0);
        is(GetLastError(), ERROR_MOD_NOT_FOUND);
        check(MiniDumpWriteDump(-1, 1, 0, 0, core::ptr::null(), core::ptr::null(), core::ptr::null()) == 0);
        is(GetLastError(), ERROR_NOT_SUPPORTED);
        SetLastError(0);
        check(SymCleanup(-1) == 1);
    }
}

// ----- crypt32

unsafe fn certs_and_memory() {
    // SAFETY: crypt32 calls with valid arguments.
    unsafe {
        // CERT_STORE_PROV_MEMORY (2) and CERT_STORE_PROV_COLLECTION (11).
        let mem = CertOpenStore(2 as *const u8, 0, 0, 0, core::ptr::null());
        let coll = CertOpenStore(11 as *const u8, 0, 0, 0, core::ptr::null());
        check(mem != 0 && coll != 0);
        check(CertAddStoreToCollection(coll, mem, 1, 0) == 1);
        // Adding it again changes nothing; a store that is no collection takes none.
        check(CertAddStoreToCollection(coll, mem, 1, 0) == 1);
        check(CertAddStoreToCollection(mem, coll, 0, 0) == 0);
        check(CertAddStoreToCollection(coll, 0x1234, 0, 0) == 0);
        is(GetLastError(), ERROR_INVALID_HANDLE);
        check(CertEnumCertificatesInStore(coll, 0) == 0);
        // CertControlStore: change notification gets an event, resync and commit succeed.
        let mut ev: isize = 0;
        check(CertControlStore(mem, 0, 2, &mut ev) == 1 && ev != 0);
        check_got(WaitForSingleObject(ev, 0) == WAIT_TIMEOUT, 1);
        CloseHandle(ev);
        check(CertControlStore(coll, 0, 1, core::ptr::null_mut()) == 1);
        check(CertControlStore(coll, 0, 3, core::ptr::null_mut()) == 1);
        check(CertControlStore(mem, 0, 99, core::ptr::null_mut()) == 0);
        // The collection keeps its own reference to the sibling.
        check(CertCloseStore(mem, 0) == 1);
        check(CertEnumCertificatesInStore(coll, 0) == 0);
        check(CertCloseStore(coll, 0) == 1);

        // CryptProtectMemory: in place, a multiple of 16 bytes, only this process can undo it.
        let plain: [u8; 32] = *b"secret secret!!!secret secret!!!";
        let mut d = plain;
        check(CryptProtectMemory(d.as_mut_ptr(), 32, 0) == 1);
        check(d != plain && d[..16] != d[16..]);
        check(CryptUnprotectMemory(d.as_mut_ptr(), 32, 0) == 1);
        check(d == plain);
        check(CryptProtectMemory(d.as_mut_ptr(), 20, 0) == 0);
        is(GetLastError(), ERROR_INVALID_PARAMETER);
        check(CryptProtectMemory(d.as_mut_ptr(), 32, 7) == 0);
    }
}

type Section = (&'static [u8], unsafe fn());

#[unsafe(no_mangle)]
pub extern "system" fn b35test_main(_peb: *mut u8) -> u32 {
    out(b"b35test: start\n");
    // SAFETY: every call is an export with valid arguments.
    unsafe {
        let sections: [Section; 6] = [
            (b"ETW without a listener", etw),
            (b"registry change notification", reg_notify),
            (b"iphlpapi", iphlp),
            (b"netapi32 and secur32", identity),
            (b"dhcpcsvc and dbghelp", dhcp_and_symbols),
            (b"crypt32 collections and protected memory", certs_and_memory),
        ];
        for (name, run) in sections {
            out(b"b35test: ");
            out(name);
            out(b"\n");
            run();
        }
        out(b"b35test: all ");
        out_num(STEP.load(Ordering::Relaxed));
        out(b" checks passed\n");
        ExitProcess(PASS)
    }
}
