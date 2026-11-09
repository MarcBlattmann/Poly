//! SetSecurityInfo with a SACL holding a mandatory label (LABEL_SECURITY_INFORMATION), the way Chromium's
//! sandbox lowers the integrity of an object; removing it with a NULL SACL; audit ACEs under SACL information;
//! labels above the caller's level. (ambient: the caller is a High integrity administrator)
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winnt.h, winbase.h, sddl.h, aclapi.h, jobapi2.h,
// processthreadsapi.h, winternl.h, ntstatus.h and winerror.h (constants,
// layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type PSID = *mut c_void;
type PVOID = *mut c_void;
type BOOL = i32;
type PACL = *mut c_void;
type PSECURITY_DESCRIPTOR = *mut c_void;
const TRUE: BOOL = 1;
const FALSE: BOOL = 0;
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

const SECURITY_MAX_SID_SIZE: usize = 68;
const WRITE_DAC: u32 = 0x0004_0000;
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_OWNER: u32 = 0x0008_0000;
const ACL_REVISION: u32 = 2;
const SYSTEM_MANDATORY_LABEL_NO_WRITE_UP: u32 = 0x1;
const SACL_SECURITY_INFORMATION: u32 = 0x8;
const LABEL_SECURITY_INFORMATION: u32 = 0x10;
const SE_KERNEL_OBJECT: u32 = 6;
const SDDL_REVISION_1: u32 = 1;
/// A `SID_IDENTIFIER_AUTHORITY` plus up to 15 sub-authorities in a buffer.
#[repr(C)]
#[derive(Clone, Copy)]
struct SidBuf {
    b: [u8; SECURITY_MAX_SID_SIZE + 4],
}

impl SidBuf {
    const fn new() -> SidBuf {
        SidBuf { b: [0; SECURITY_MAX_SID_SIZE + 4] }
    }
    fn as_psid(&mut self) -> PSID {
        self.b.as_mut_ptr().cast()
    }
    fn as_const(&self) -> *const c_void {
        self.b.as_ptr().cast()
    }
    /// The bytes of the SID (its length from the header).
    fn bytes(&self) -> &[u8] {
        let n = 8 + 4 * usize::from(self.b[1]);
        &self.b[..n.min(self.b.len())]
    }
}

/// winbase.h: `SECURITY_ATTRIBUTES`.
#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    descriptor: PVOID,
    inherit: BOOL,
}

/// An ACL in an 8-aligned buffer.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct AclBuf {
    b: [u8; 1024],
}

impl AclBuf {
    const fn new() -> AclBuf {
        AclBuf { b: [0; 1024] }
    }
    fn p(&mut self) -> PACL {
        self.b.as_mut_ptr().cast()
    }
    /// The ACL's bytes by its own size field.
    fn bytes(&self) -> &[u8] {
        let n = usize::from(u16::from_le_bytes([self.b[2], self.b[3]]));
        &self.b[..n.min(self.b.len())]
    }
}

/// The last-error code after a failed call, as a plain number.
fn gle() -> u32 {
    // SAFETY: reads the calling thread's last error.
    unsafe { GetLastError() }
}

fn clear_error() {
    // SAFETY: sets the calling thread's last error.
    unsafe { SetLastError(0xDEAD) };
}

/// A NUL-terminated UTF-16 copy of an ASCII string (up to 255 units).
fn wide(s: &str) -> [u16; 256] {
    let mut w = [0u16; 256];
    for (i, c) in s.bytes().enumerate().take(255) {
        w[i] = u16::from(c);
    }
    w
}

/// The units of a NUL-terminated UTF-16 string.
fn wide_len(p: *const u16) -> usize {
    let mut n = 0;
    // SAFETY: the caller passes a NUL-terminated string.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
    }
    n
}

/// A UTF-16 string pointer as text (ASCII only; others as `?`).
fn wide_text<const N: usize>(p: *const u16) -> Buf<N> {
    let mut b = Buf::<N>::new();
    if p.is_null() {
        let _ = write!(b, "(null)");
        return b;
    }
    let n = wide_len(p);
    for i in 0..n {
        // SAFETY: i < n, inside the string.
        let c = unsafe { *p.add(i) };
        let _ = write!(b, "{}", if c < 0x80 { c as u8 as char } else { '?' });
    }
    b
}

/// The SID in the text form of `ConvertSidToStringSidW`, or an error marker.
fn sid_text(sid: *const c_void) -> Buf<200> {
    let mut out: *mut u16 = core::ptr::null_mut();
    // SAFETY: `sid` is a valid SID or the call fails; `out` is a valid out pointer.
    let ok = unsafe { ConvertSidToStringSidW(sid, &mut out) };
    if ok == 0 {
        let mut b = Buf::<200>::new();
        let _ = write!(b, "<invalid sid>");
        return b;
    }
    let b = wide_text::<200>(out);
    // SAFETY: the string was allocated by the call with LocalAlloc.
    unsafe { LocalFree(out.cast()) };
    b
}

/// A SID from its string form (up to 68 bytes), through ConvertStringSidToSidW.
fn sid_of(s: &str) -> SidBuf {
    let w = wide(s);
    let mut p: PSID = core::ptr::null_mut();
    let mut out = SidBuf::new();
    // SAFETY: `w` is NUL-terminated; `p` receives a LocalAlloc'ed SID, copied and freed below.
    unsafe {
        if ConvertStringSidToSidW(w.as_ptr(), &mut p) != 0 {
            let n = GetLengthSid(p) as usize;
            core::ptr::copy_nonoverlapping(p.cast::<u8>(), out.b.as_mut_ptr(), n.min(out.b.len()));
            LocalFree(p);
        }
    }
    out
}

/// The bytes of the little-endian u32 at `off`.
fn le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// The last error after a call, if it failed (`ok` is 0); -1 after a success,
/// where the last error is not specified.
fn gle_if(ok: i32) -> i64 {
    if ok == 0 { i64::from(gle()) } else { -1 }
}

/// The text of a SID with the machine-dependent parts masked: S-1-5-21-a-b-c-RID prints as
/// S-1-5-21-<machine>-RID and a logon-session SID S-1-5-5-x-y as S-1-5-5-<logon>.
fn mask_sid(sid: *const c_void) -> Buf<200> {
    let full = sid_text(sid);
    let s = full.as_str();
    let mut b = Buf::<200>::new();
    if s.starts_with("S-1-5-21-") && s.matches('-').count() >= 7 {
        let rid = s.rsplit('-').next().unwrap_or("?");
        if rid.parse::<u32>().map_or(false, |r| r >= 1000) {
            let _ = write!(b, "S-1-5-21-<machine>-<account>");
        } else {
            let _ = write!(b, "S-1-5-21-<machine>-{rid}");
        }
    } else if s.starts_with("S-1-5-5-") {
        let _ = write!(b, "S-1-5-5-<logon>");
    } else {
        let _ = write!(b, "{s}");
    }
    b
}

/// `say!` with a longer line (SDDL texts, hex dumps).
macro_rules! sdsay {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<2000>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

const SD_SENTINEL: usize = 0x1234_5678_9ABC;
/// `s` with the machine-dependent SIDs masked: S-1-5-21-<big>-<big>-<big>-RID becomes
/// S-1-5-21-<machine>-RID and S-1-5-5-x-y becomes S-1-5-5-<logon>. Small made-up domain numbers
/// (S-1-5-21-1-2-3-...) are left alone.
fn sd_mask_text(s: &str) -> Buf<1500> {
    let b = s.as_bytes();
    let mut out = Buf::<1500>::new();
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"S-1-5-21-") {
            let mut j = i + 9;
            let mut groups = 0;
            let mut first_len = 0;
            let mut last = (j, j);
            loop {
                let st = j;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                if j == st {
                    if groups > 0 && b[st - 1] == b'-' {
                        j = st - 1;
                    }
                    break;
                }
                groups += 1;
                if groups == 1 {
                    first_len = j - st;
                }
                last = (st, j);
                if j < b.len() && b[j] == b'-' {
                    j += 1;
                } else {
                    break;
                }
            }
            if first_len >= 5 && groups >= 3 {
                let _ = write!(out, "S-1-5-21-<machine>");
                if groups >= 4 {
                    let _ = write!(out, "-{}", core::str::from_utf8(&b[last.0..last.1]).unwrap_or("?"));
                }
                i = j;
                continue;
            }
        }
        if b[i..].starts_with(b"S-1-5-5-") {
            let mut j = i + 8;
            let mut groups = 0;
            loop {
                let st = j;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                if j == st {
                    break;
                }
                groups += 1;
                if groups < 2 && j < b.len() && b[j] == b'-' {
                    j += 1;
                } else {
                    break;
                }
            }
            if groups == 2 {
                let _ = write!(out, "S-1-5-5-<logon>");
                i = j;
                continue;
            }
        }
        let _ = write!(out, "{}", b[i] as char);
        i += 1;
    }
    out
}

/// The result of ConvertSecurityDescriptorToStringSecurityDescriptorW: success, the text (machine
/// SIDs masked), the length it reported (characters including the NUL?), the real string length and the
/// error after a failure.
struct SdText {
    ok: i32,
    text: Buf<1500>,
    len: u32,
    units: usize,
    err: i64,
}

fn sd_sddl_rev(sd: *const c_void, rev: u32, info: u32) -> SdText {
    let mut out: *mut u16 = core::ptr::null_mut();
    let mut len = 0xAAAA_AAAAu32;
    clear_error();
    // SAFETY: `sd` is a descriptor or the call fails; both out pointers are valid.
    let ok = unsafe { ConvertSecurityDescriptorToStringSecurityDescriptorW(sd, rev, info, &mut out, &mut len) };
    let err = gle_if(ok);
    let mut text = Buf::<1500>::new();
    let mut units = 0;
    if ok != 0 && !out.is_null() {
        let raw = wide_text::<1500>(out);
        text = sd_mask_text(raw.as_str());
        units = wide_len(out);
        // SAFETY: the string was allocated with LocalAlloc by the call.
        unsafe { LocalFree(out.cast()) };
    } else {
        let _ = write!(text, "-");
    }
    SdText { ok, text, len, units, err }
}

fn sd_sddl(sd: *const c_void, info: u32) -> SdText {
    sd_sddl_rev(sd, SDDL_REVISION_1, info)
}

/// One line: success, text, reported length and string length.
fn sd_sddl_say(t: &mut Test, label: &str, sd: *const c_void, info: u32) {
    let r = sd_sddl(sd, info);
    if r.ok != 0 {
        sdsay!(t, "{label}: ok=1 text={} len={} units={}", r.text.as_str(), r.len, r.units);
    } else {
        sdsay!(t, "{label}: ok=0 err={} len={:#x}", r.err, r.len);
    }
}

/// An ACL's header and ACEs from a pointer: "rev size count" then per ACE "type flags size mask sid".
fn sd_acl_show(t: &mut Test, label: &str, acl: *const c_void) {
    if acl.is_null() {
        sdsay!(t, "{label}: (null)");
        return;
    }
    if acl as usize == SD_SENTINEL {
        sdsay!(t, "{label}: (not returned)");
        return;
    }
    // SAFETY: the caller passes a pointer to an ACL header (8 bytes at least).
    let head = unsafe { core::slice::from_raw_parts(acl as *const u8, 8) };
    let size = usize::from(u16::from_le_bytes([head[2], head[3]])).min(1024);
    let count = usize::from(u16::from_le_bytes([head[4], head[5]]));
    sdsay!(
        t,
        "{label}: acl rev={} sbz1={} size={} count={count} sbz2={}",
        head[0],
        head[1],
        size,
        u16::from_le_bytes([head[6], head[7]])
    );
    // SAFETY: the ACL is `size` bytes (as its own header says, at most 1024 here).
    let b = unsafe { core::slice::from_raw_parts(acl as *const u8, size.max(8)) };
    let mut off = 8usize;
    for i in 0..count.min(30) {
        if off + 8 > b.len() {
            sdsay!(t, "{label}: ace {i} at {off} is outside the ACL");
            break;
        }
        let ty = b[off];
        let flags = b[off + 1];
        let sz = usize::from(u16::from_le_bytes([b[off + 2], b[off + 3]]));
        let mask = le32(b, off + 4);
        let sid_off = if (5..=8).contains(&ty) && off + 12 <= b.len() {
            let f = le32(b, off + 8);
            off + 12 + if f & 1 != 0 { 16 } else { 0 } + if f & 2 != 0 { 16 } else { 0 }
        } else {
            off + 8
        };
        let sid = if sid_off + 8 <= b.len() { mask_sid(b[sid_off..].as_ptr().cast()) } else { Buf::<200>::new() };
        sdsay!(
            t,
            "{label}: ace {i} off={off} type={ty:#x} flags={flags:#x} size={sz} mask={mask:#x} sid={}",
            sid.as_str()
        );
        if sz == 0 {
            break;
        }
        off += sz;
    }
}

/// The pointer `p` against the descriptor at `base` (`len` bytes): null, the offset inside, the
/// untouched sentinel or elsewhere.
fn sd_where(p: *const c_void, base: *const c_void, len: usize) -> Buf<40> {
    let mut b = Buf::<40>::new();
    if p.is_null() {
        let _ = write!(b, "null");
    } else if p as usize == SD_SENTINEL {
        let _ = write!(b, "unchanged");
    } else {
        let d = (p as usize).wrapping_sub(base as usize);
        if d < len {
            let _ = write!(b, "inside+{d}");
        } else {
            let _ = write!(b, "elsewhere");
        }
    }
    b
}

/// Builds an ACL into `acl` from (ace type, ace flags, mask, sid text). Types: 0 allowed, 1 denied,
/// 2 audit (success and failure), 0x11 mandatory label (the mask is the label policy).
fn sd_build_acl(acl: &mut AclBuf, aces: &[(u8, u8, u32, &str)]) -> bool {
    acl.b.fill(0);
    let mut size = 8usize;
    for &(_, _, _, sid) in aces {
        size += 8 + sid_of(sid).bytes().len();
    }
    size = (size + 3) & !3;
    // SAFETY: `acl` is 1024 writable bytes and `size` is the room the ACEs need.
    if unsafe { InitializeAcl(acl.p(), size as u32, ACL_REVISION) } == 0 {
        return false;
    }
    for &(ty, flags, mask, sid) in aces {
        let s = sid_of(sid);
        // SAFETY: the ACL is initialized and has room; `s` is a valid SID.
        let ok = unsafe {
            match ty {
                0 => AddAccessAllowedAceEx(acl.p(), ACL_REVISION, u32::from(flags), mask, s.as_const()),
                1 => AddAccessDeniedAceEx(acl.p(), ACL_REVISION, u32::from(flags), mask, s.as_const()),
                2 => AddAuditAccessAceEx(acl.p(), ACL_REVISION, u32::from(flags), mask, s.as_const(), TRUE, TRUE),
                0x11 => AddMandatoryAce(acl.p(), ACL_REVISION, u32::from(flags), mask, s.as_const()),
                _ => 0,
            }
        };
        if ok == 0 {
            return false;
        }
    }
    true
}

/// A unique object name "PolySecSd<pid>_<n>" (UTF-16, NUL-terminated).
fn sd_unique_name() -> [u16; 256] {
    static COUNTER: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let mut text = Buf::<80>::new();
    // SAFETY: reads the process id.
    let _ = write!(text, "PolySecSd{}_{}", unsafe { GetCurrentProcessId() }, n);
    wide(text.as_str())
}

/// A new named manual-reset event (unique name) with the default security of the caller's token.
fn sd_event() -> HANDLE {
    let name = sd_unique_name();
    // SAFETY: no attributes, NUL-terminated name.
    unsafe { CreateEventW(core::ptr::null(), TRUE, FALSE, name.as_ptr()) }
}

/// A copy of a handle with exactly `access`.
fn sd_dup_handle(h: HANDLE, access: u32) -> HANDLE {
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: duplicates within the current process.
    unsafe { DuplicateHandle(GetCurrentProcess(), h, GetCurrentProcess(), &mut out, access, FALSE, 0) };
    out
}

/// The outputs of GetSecurityInfo (all pointers start as the sentinel); the descriptor is freed on drop.
struct SdGsi {
    rc: u32,
    owner: *mut c_void,
    group: *mut c_void,
    dacl: *mut c_void,
    sacl: *mut c_void,
    sd: *mut c_void,
}

impl Drop for SdGsi {
    fn drop(&mut self) {
        if !self.sd.is_null() && self.sd as usize != SD_SENTINEL {
            // SAFETY: allocated with LocalAlloc by GetSecurityInfo.
            unsafe { LocalFree(self.sd) };
        }
    }
}

/// GetSecurityInfo with the output pointers selected by `give`: 1 owner, 2 group, 4 DACL, 8 SACL, 0x10 descriptor.
fn sd_gsi(h: HANDLE, kind: u32, info: u32, give: u32) -> SdGsi {
    let s = SD_SENTINEL as *mut c_void;
    let mut r = SdGsi { rc: 0xAAAA, owner: s, group: s, dacl: s, sacl: s, sd: s };
    let np = core::ptr::null_mut::<*mut c_void>();
    let po = if give & 1 != 0 { &mut r.owner as *mut *mut c_void } else { np };
    let pg = if give & 2 != 0 { &mut r.group as *mut *mut c_void } else { np };
    let pd = if give & 4 != 0 { &mut r.dacl as *mut *mut c_void } else { np };
    let ps = if give & 8 != 0 { &mut r.sacl as *mut *mut c_void } else { np };
    let psd = if give & 0x10 != 0 { &mut r.sd as *mut *mut c_void } else { np };
    // SAFETY: the out pointers are NULL or point into `r`.
    r.rc = unsafe { GetSecurityInfo(h, kind, info, po, pg, pd, ps, psd) };
    r
}

/// SetSecurityInfo for a kernel object with pointers as given (NULL allowed): the return code.
fn sd_ssi(h: HANDLE, info: u32, owner: *mut c_void, group: *mut c_void, dacl: *mut c_void, sacl: *mut c_void) -> u32 {
    // SAFETY: the pointers are NULL or valid SIDs/ACLs.
    unsafe { SetSecurityInfo(h, SE_KERNEL_OBJECT, info, owner, group, dacl, sacl) }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn LocalFree(p: PVOID) -> PVOID;
    fn GetCurrentProcess() -> HANDLE;
    fn GetCurrentProcessId() -> u32;
    fn CreateEventW(attrs: *const SECURITY_ATTRIBUTES, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn DuplicateHandle(
        src_process: HANDLE,
        src: HANDLE,
        dst_process: HANDLE,
        dst: *mut HANDLE,
        access: u32,
        inherit: BOOL,
        options: u32,
    ) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
        sd: *const c_void,
        revision: u32,
        info: u32,
        out: *mut *mut u16,
        len: *mut u32,
    ) -> BOOL;
    fn InitializeAcl(acl: PACL, len: u32, revision: u32) -> BOOL;
    fn AddAccessAllowedAceEx(acl: PACL, revision: u32, flags: u32, mask: u32, sid: *const c_void) -> BOOL;
    fn AddAccessDeniedAceEx(acl: PACL, revision: u32, flags: u32, mask: u32, sid: *const c_void) -> BOOL;
    fn AddAuditAccessAceEx(
        acl: PACL,
        revision: u32,
        flags: u32,
        mask: u32,
        sid: *const c_void,
        success: BOOL,
        failure: BOOL,
    ) -> BOOL;
    fn AddMandatoryAce(acl: PACL, revision: u32, flags: u32, policy: u32, sid: *const c_void) -> BOOL;
    fn GetSecurityInfo(
        h: HANDLE,
        kind: u32,
        info: u32,
        owner: *mut PSID,
        group: *mut PSID,
        dacl: *mut PACL,
        sacl: *mut PACL,
        sd: *mut PSECURITY_DESCRIPTOR,
    ) -> u32;
    fn SetSecurityInfo(h: HANDLE, kind: u32, info: u32, owner: PSID, group: PSID, dacl: PACL, sacl: PACL) -> u32;
}

poly_wtest::main!(test);

fn label_back(t: &mut Test, label: &str, h: HANDLE) {
    let r = sd_gsi(h, SE_KERNEL_OBJECT, LABEL_SECURITY_INFORMATION, 0x1F);
    sdsay!(t, "{label}: rc={}", r.rc);
    if r.rc == 0 {
        sdsay!(t, "  sacl output: {}", sd_where(r.sacl, r.sd, 4096).as_str());
        sd_acl_show(t, "  sacl", r.sacl);
        sd_sddl_say(t, "  text", r.sd, LABEL_SECURITY_INFORMATION);
    }
}

fn test(t: &mut Test) {
    let ev = sd_event();
    let np = core::ptr::null_mut();
    label_back(t, "initial", ev);
    for (title, aces) in [
        ("Low, NO_WRITE_UP", &[(0x11u8, 0u8, SYSTEM_MANDATORY_LABEL_NO_WRITE_UP, "S-1-16-4096")][..]),
        ("Low, NO_WRITE_UP|NO_READ_UP|NO_EXECUTE_UP", &[(0x11, 0, 7, "S-1-16-4096")][..]),
        ("Medium, NO_WRITE_UP", &[(0x11, 0, 1, "S-1-16-8192")][..]),
        ("High, NO_WRITE_UP", &[(0x11, 0, 1, "S-1-16-12288")][..]),
        ("System, NO_WRITE_UP", &[(0x11, 0, 1, "S-1-16-16384")][..]),
        ("Untrusted, NO_WRITE_UP", &[(0x11, 0, 1, "S-1-16-0")][..]),
        ("Low, policy 0", &[(0x11, 0, 0, "S-1-16-4096")][..]),
        ("label RID 4097", &[(0x11, 0, 1, "S-1-16-4097")][..]),
        ("two labels", &[(0x11, 0, 1, "S-1-16-4096"), (0x11, 0, 1, "S-1-16-8192")][..]),
        ("an audit ACE only", &[(2, 0xC0, 1, "S-1-1-0")][..]),
        ("empty SACL", &[][..]),
    ] {
        let mut sacl = AclBuf::new();
        if !sd_build_acl(&mut sacl, aces) {
            sdsay!(t, "{title}: could not build the ACL");
            continue;
        }
        sdsay!(t, "set SACL [{title}] with LABEL: rc={}", sd_ssi(ev, LABEL_SECURITY_INFORMATION, np, np, np, sacl.p()));
        label_back(t, "  after", ev);
    }
    sdsay!(t, "set NULL SACL with LABEL: rc={}", sd_ssi(ev, LABEL_SECURITY_INFORMATION, np, np, np, np));
    label_back(t, "  after", ev);
    let mut sacl = AclBuf::new();
    sd_build_acl(&mut sacl, &[(0x11, 0, 1, "S-1-16-4096")]);
    sdsay!(t, "set label Low again: rc={}", sd_ssi(ev, LABEL_SECURITY_INFORMATION, np, np, np, sacl.p()));
    sdsay!(t, "set SACL with SACL information: rc={}", sd_ssi(ev, SACL_SECURITY_INFORMATION, np, np, np, sacl.p()));
    sdsay!(t, "set NULL SACL with SACL information: rc={}", sd_ssi(ev, SACL_SECURITY_INFORMATION, np, np, np, np));
    sdsay!(
        t,
        "set SACL with SACL|LABEL information: rc={}",
        sd_ssi(ev, SACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION, np, np, np, sacl.p())
    );
    let mut audit = AclBuf::new();
    sd_build_acl(&mut audit, &[(2, 0xC0, 1, "S-1-1-0")]);
    sdsay!(
        t,
        "set audit-only SACL with SACL information: rc={}",
        sd_ssi(ev, SACL_SECURITY_INFORMATION, np, np, np, audit.p())
    );
    label_back(t, "  after", ev);
    // A handle without WRITE_OWNER (needed for the label).
    let wdac = sd_dup_handle(ev, WRITE_DAC | READ_CONTROL);
    sdsay!(
        t,
        "set label through a WRITE_DAC handle: rc={}",
        sd_ssi(wdac, LABEL_SECURITY_INFORMATION, np, np, np, sacl.p())
    );
    let wown = sd_dup_handle(ev, WRITE_OWNER | READ_CONTROL);
    sdsay!(
        t,
        "set label through a WRITE_OWNER handle: rc={}",
        sd_ssi(wown, LABEL_SECURITY_INFORMATION, np, np, np, sacl.p())
    );
    // SAFETY: closing the handles.
    unsafe {
        CloseHandle(wown);
        CloseHandle(wdac);
        CloseHandle(ev);
    }
}
