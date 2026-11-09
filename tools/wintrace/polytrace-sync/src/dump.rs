//! Out-buffer dumps of `polytrace-sync` (spec-chrome-win-3).
//!
//! After a traced call returns, [`dump`] reads what the call left in memory (strings, structures,
//! registry data, out handles and pointers) and writes it as an `"out"` array on the record, so a
//! run on real Windows and a run on Poly can be compared value by value, not only call by call.
//! Handles and pointers are written as *classes* (`null`, `-1`, `handle`, `stack`, `private`,
//! `mapped`, `image:<module>+0x<offset>`, ...), never as values: a value differs from run to run
//! (ASLR) and from system to system.
//!
//! Every read of the program's memory goes through `ReadProcessMemory` on the own process, which
//! reports an invalid address instead of faulting, so a wrong pointer in a program under trace can
//! not crash the logger. Everything written here is observed from the program's own memory at the
//! API boundary: nothing of any system DLL is looked at.
//!
//! The documented structures read are `UNICODE_STRING`, `OBJECT_ATTRIBUTES`,
//! `KEY_VALUE_BASIC/FULL/PARTIAL_INFORMATION` (`wdm.h`, `winternl.h`), `SYSTEM_INFO`,
//! `OSVERSIONINFOEXW`, `STARTUPINFOW`, `MEMORY_BASIC_INFORMATION`, `CPINFO`,
//! `RTL_CRITICAL_SECTION` (`winnt.h`, `sysinfoapi.h`, `processthreadsapi.h`, `memoryapi.h`,
//! `winnls.h`) and `VS_FIXEDFILEINFO` (`verrsrc.h`); the layouts are those of the SDK headers.

use super::{W, lock, unlock};
use core::cell::UnsafeCell;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ReadProcessMemory(process: isize, base: *const u8, buf: *mut u8, size: usize, read: *mut usize) -> i32;
    fn VirtualQuery(addr: *const u8, info: *mut Mbi, len: usize) -> usize;
    fn GetModuleHandleExW(flags: u32, addr: *const u16, module: *mut isize) -> i32;
    fn GetModuleFileNameW(module: isize, buf: *mut u16, size: u32) -> u32;
}

/// `MEMORY_BASIC_INFORMATION` (x64, `winnt.h`): 48 bytes.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Mbi {
    base: u64,
    alloc_base: u64,
    alloc_protect: u32,
    partition: u16,
    pad0: u16,
    size: u64,
    state: u32,
    protect: u32,
    kind: u32,
    pad1: u32,
}

/// `GetCurrentProcess()`'s pseudo handle.
const SELF: isize = -1;
/// `MEM_IMAGE`, `MEM_MAPPED`, `MEM_PRIVATE` (`winnt.h`).
const MEM_IMAGE: u32 = 0x100_0000;
const MEM_MAPPED: u32 = 0x4_0000;
const MEM_PRIVATE: u32 = 0x2_0000;
/// `GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT`.
const FROM_ADDRESS: u32 = 0x4 | 0x2;

// ----- reading the program's memory ----------------------------------------------------------

/// Copy up to `out.len()` bytes of this process' memory at `addr`; returns how many were readable.
pub fn peek(addr: u64, out: &mut [u8]) -> usize {
    let mut got = 0;
    while got < out.len() {
        let a = addr.wrapping_add(got as u64);
        let to_page = 0x1000 - (a & 0xFFF) as usize;
        let n = (out.len() - got).min(to_page).min(256);
        let mut read = 0usize;
        // SAFETY: `ReadProcessMemory` on the own process checks the source and reports failure
        // instead of faulting; the destination is `n` writable bytes inside `out`.
        let ok = unsafe { ReadProcessMemory(SELF, a as *const u8, out.as_mut_ptr().add(got), n, &mut read) };
        got += read.min(n);
        if ok == 0 || read < n {
            break;
        }
    }
    got
}

fn rd16(a: u64) -> Option<u16> {
    let mut b = [0u8; 2];
    (peek(a, &mut b) == 2).then(|| u16::from_le_bytes(b))
}
fn rd32(a: u64) -> Option<u32> {
    let mut b = [0u8; 4];
    (peek(a, &mut b) == 4).then(|| u32::from_le_bytes(b))
}
fn rd64(a: u64) -> Option<u64> {
    let mut b = [0u8; 8];
    (peek(a, &mut b) == 8).then(|| u64::from_le_bytes(b))
}

/// A NUL-terminated UTF-16 string of at most `out.len()` units: `(units, ended with NUL)`.
fn rd_wide(addr: u64, out: &mut [u16]) -> Option<(usize, bool)> {
    let mut n = 0;
    while n < out.len() {
        let mut b = [0u8; 64];
        let want = ((out.len() - n) * 2).min(64);
        let got = peek(addr.wrapping_add(n as u64 * 2), &mut b[..want]);
        if got < 2 {
            return if n == 0 { None } else { Some((n, false)) };
        }
        for k in 0..got / 2 {
            let u = u16::from_le_bytes([b[2 * k], b[2 * k + 1]]);
            if u == 0 {
                return Some((n, true));
            }
            out[n] = u;
            n += 1;
        }
        if got < want {
            return Some((n, false));
        }
    }
    Some((n, false))
}

/// Exactly `count` UTF-16 units (a counted string); fewer when the memory ends.
fn rd_wide_n(addr: u64, count: usize, out: &mut [u16]) -> usize {
    let count = count.min(out.len());
    let mut n = 0;
    while n < count {
        let mut b = [0u8; 64];
        let want = ((count - n) * 2).min(64);
        let got = peek(addr.wrapping_add(n as u64 * 2), &mut b[..want]);
        for k in 0..got / 2 {
            out[n] = u16::from_le_bytes([b[2 * k], b[2 * k + 1]]);
            n += 1;
        }
        if got < want {
            break;
        }
    }
    n
}

/// A NUL-terminated ANSI string of at most `out.len()` bytes.
fn rd_ansi(addr: u64, out: &mut [u8]) -> Option<(usize, bool)> {
    let mut n = 0;
    while n < out.len() {
        let want = (out.len() - n).min(64);
        let got = peek(addr.wrapping_add(n as u64), &mut out[n..n + want]);
        if got == 0 {
            return if n == 0 { None } else { Some((n, false)) };
        }
        if let Some(z) = out[n..n + got].iter().position(|&b| b == 0) {
            return Some((n + z, true));
        }
        n += got;
        if got < want {
            return Some((n, false));
        }
    }
    Some((n, false))
}

// ----- classes of values --------------------------------------------------------------------

const MODS: usize = 64;
const NAME: usize = 32;

struct Mods {
    base: [u64; MODS],
    len: [u8; MODS],
    name: [[u8; NAME]; MODS],
    next: usize,
}

struct Cache(UnsafeCell<Mods>);
// SAFETY: every access is under the logger's critical section ([`lock`]).
unsafe impl Sync for Cache {}

static MODCACHE: Cache =
    Cache(UnsafeCell::new(Mods { base: [0; MODS], len: [0; MODS], name: [[0; NAME]; MODS], next: 0 }));

/// The image `addr` lies in: its base and the slot of its lower-case base name in the cache.
fn module_of(addr: u64) -> Option<(u64, usize)> {
    if addr < 0x10000 {
        return None;
    }
    let mut h = 0isize;
    // SAFETY: `h` is writable; with FROM_ADDRESS the "name" argument is an address, never read as a string.
    let ok = unsafe { GetModuleHandleExW(FROM_ADDRESS, addr as *const u16, &mut h) };
    let mut base = if ok != 0 { h as u64 } else { 0 };
    if base == 0 {
        let mut m = Mbi::default();
        // SAFETY: `m` is a writable 48-byte structure.
        let r = unsafe { VirtualQuery(addr as *const u8, &mut m, core::mem::size_of::<Mbi>()) };
        if r == core::mem::size_of::<Mbi>() && m.kind == MEM_IMAGE {
            base = m.alloc_base;
        }
    }
    if base == 0 {
        return None;
    }
    lock();
    // SAFETY: under the lock.
    let c = unsafe { &mut *MODCACHE.0.get() };
    let mut found = None;
    for i in 0..MODS {
        if c.base[i] == base && c.len[i] != 0 {
            found = Some(i);
            break;
        }
    }
    let slot = match found {
        Some(i) => i,
        None => {
            let mut buf = [0u16; 300];
            // SAFETY: `buf` holds 300 units.
            let n = (unsafe { GetModuleFileNameW(base as isize, buf.as_mut_ptr(), 300) } as usize).min(299);
            let mut start = 0;
            for (i, &u) in buf[..n].iter().enumerate() {
                if u == b'\\' as u16 || u == b'/' as u16 {
                    start = i + 1;
                }
            }
            let slot = c.next % MODS;
            c.next += 1;
            let mut l = 0;
            for &u in &buf[start..n] {
                if l < NAME {
                    c.name[slot][l] = if u < 0x80 { (u as u8).to_ascii_lowercase() } else { b'?' };
                    l += 1;
                }
            }
            if l == 0 {
                c.name[slot][0] = b'?';
                l = 1;
            }
            c.base[slot] = base;
            c.len[slot] = l as u8;
            slot
        }
    };
    unlock();
    Some((base, slot))
}

/// `"name"` (the base file name, lower case) of a cache slot, appended to `w` without quotes.
fn put_mod_name(w: &mut W, slot: usize) {
    lock();
    // SAFETY: under the lock.
    let c = unsafe { &*MODCACHE.0.get() };
    let l = c.len[slot] as usize;
    let name = c.name[slot];
    unlock();
    w.bytes(&name[..l]);
}

/// The caller's module and offset: `"rm":"chrome_elf.dll","ro":"0x8e1f5"` (written after `ra`).
pub fn put_caller(w: &mut W, ra: u64) {
    if let Some((base, slot)) = module_of(ra) {
        w.bytes(b",\"rm\":\"");
        put_mod_name(w, slot);
        w.bytes(b"\",\"ro\":");
        w.hex(ra - base);
    }
}

/// This thread's stack: `(low, high)`, from the TEB (`NT_TIB.StackBase`, `StackLimit`, and
/// `DeallocationStack` when set).
fn stack_range() -> (u64, u64) {
    let (hi, lim, dealloc): (u64, u64, u64);
    // SAFETY: `gs:[0]` is the TEB; offsets 8, 0x10 and 0x1478 are `StackBase`, `StackLimit`,
    // `DeallocationStack` (winternl.h / the public TEB layout).
    unsafe {
        core::arch::asm!(
            "mov {0}, gs:[8]",
            "mov {1}, gs:[0x10]",
            "mov {2}, gs:[0x1478]",
            out(reg) hi, out(reg) lim, out(reg) dealloc,
            options(nostack, readonly, preserves_flags)
        );
    }
    let mut lo = lim;
    if dealloc != 0 && dealloc < lo {
        lo = dealloc;
    }
    (lo, hi)
}

/// Write `"<class>"` of a pointer value.
pub fn pclass(w: &mut W, v: u64) {
    w.byte(b'"');
    pclass_body(w, v);
    w.byte(b'"');
}

fn pclass_body(w: &mut W, v: u64) {
    if v == 0 {
        return w.bytes(b"null");
    }
    if v == u64::MAX {
        return w.bytes(b"-1");
    }
    if v < 0x10000 {
        w.bytes(b"int:");
        return w.dec(v);
    }
    if v >> 47 != 0 {
        return w.bytes(b"noncanonical");
    }
    let (lo, hi) = stack_range();
    if lo != 0 && v >= lo && v < hi {
        return w.bytes(b"stack");
    }
    if let Some((base, slot)) = module_of(v) {
        w.bytes(b"image:");
        put_mod_name(w, slot);
        w.bytes(b"+0x");
        hex_plain(w, v - base);
        return;
    }
    let mut m = Mbi::default();
    // SAFETY: `m` is a writable 48-byte structure.
    let r = unsafe { VirtualQuery(v as *const u8, &mut m, core::mem::size_of::<Mbi>()) };
    if r != core::mem::size_of::<Mbi>() {
        return w.bytes(b"unmapped");
    }
    if m.state != 0x1000 {
        return w.bytes(b"reserved-or-free");
    }
    w.bytes(match m.kind {
        MEM_IMAGE => b"image" as &[u8],
        MEM_MAPPED => b"mapped",
        MEM_PRIVATE => b"private",
        _ => b"other",
    });
}

/// Write `"<class>"` of a handle value: `null`, `-1`, `-2` (the pseudo handles) or `handle`.
pub fn hclass(w: &mut W, v: u64) {
    w.bytes(match v {
        0 => b"\"null\"" as &[u8],
        u64::MAX => b"\"-1\"",
        0xFFFF_FFFF_FFFF_FFFE => b"\"-2\"",
        0xFFFF_FFFF => b"\"-1\"",
        _ => b"\"handle\"",
    });
}

/// `0x`-less lower-case hex.
fn hex_plain(w: &mut W, v: u64) {
    let mut started = false;
    for shift in (0..16).rev() {
        let n = ((v >> (shift * 4)) & 0xF) as u8;
        if n != 0 || started || shift == 0 {
            started = true;
            w.byte(if n < 10 { b'0' + n } else { b'a' + n - 10 });
        }
    }
}

fn sdec(w: &mut W, v: i64) {
    if v < 0 {
        w.byte(b'-');
        w.dec(v.unsigned_abs());
    } else {
        w.dec(v as u64);
    }
}

// ----- strings and bytes --------------------------------------------------------------------

/// A JSON string of UTF-16 units (`…` when `more`).
fn put_wide(w: &mut W, s: &[u16], more: bool) {
    w.byte(b'"');
    let mut i = 0;
    while i < s.len() {
        let u = s[i] as u32;
        let c = if (0xD800..0xDC00).contains(&u) {
            if i + 1 < s.len() && (0xDC00..0xE000).contains(&(s[i + 1] as u32)) {
                i += 1;
                0x10000 + ((u - 0xD800) << 10) + (s[i] as u32 - 0xDC00)
            } else {
                0xFFFD
            }
        } else if (0xDC00..0xE000).contains(&u) {
            0xFFFD
        } else {
            u
        };
        w.json_char(c);
        i += 1;
    }
    if more {
        w.json_char(0x2026);
    }
    w.byte(b'"');
}

fn put_ansi(w: &mut W, s: &[u8], more: bool) {
    w.byte(b'"');
    for &b in s {
        w.json_char(b as u32);
    }
    if more {
        w.json_char(0x2026);
    }
    w.byte(b'"');
}

/// `"<hex>"` of the first `shown` bytes.
fn put_hex_bytes(w: &mut W, b: &[u8]) {
    w.byte(b'"');
    for &x in b {
        for n in [x >> 4, x & 15] {
            w.byte(if n < 10 { b'0' + n } else { b'a' + n - 10 });
        }
    }
    w.byte(b'"');
}

// ----- the dump builder ---------------------------------------------------------------------

/// The `"out"` array under construction.
pub struct D<'a> {
    pub w: &'a mut W,
    n: u32,
}

/// `idx` of the value that is the call's result.
const RET: i8 = -1;

impl<'a> D<'a> {
    pub fn new(w: &'a mut W) -> Self {
        D { w, n: 0 }
    }
    fn open(&mut self, idx: i8) {
        if self.n > 0 {
            self.w.byte(b',');
        }
        self.n += 1;
        self.w.bytes(b"{\"a\":");
        if idx < 0 {
            self.w.bytes(b"\"ret\"");
        } else {
            self.w.dec(idx as u64);
        }
    }
    fn close(&mut self) {
        self.w.byte(b'}');
    }
    fn key(&mut self, k: &[u8]) {
        self.w.bytes(b",\"");
        self.w.bytes(k);
        self.w.bytes(b"\":");
    }
    fn unreadable(&mut self, idx: i8) {
        self.open(idx);
        self.w.bytes(b",\"unreadable\":1");
        self.close();
    }

    /// A pointer's (or pointer-sized value's) class.
    fn cls(&mut self, idx: i8, v: u64) {
        self.open(idx);
        self.key(b"c");
        pclass(self.w, v);
        self.close();
    }
    fn hcls(&mut self, idx: i8, v: u64) {
        self.open(idx);
        self.key(b"h");
        hclass(self.w, v);
        self.close();
    }
    /// The pointer-sized value `*p` as a class (an out pointer) or handle class.
    fn ptr_out(&mut self, idx: i8, p: u64, handle: bool) {
        if p == 0 {
            return;
        }
        match rd64(p) {
            Some(v) if handle => self.hcls(idx, v),
            Some(v) => self.cls(idx, v),
            None => self.unreadable(idx),
        }
    }
    fn u32_out(&mut self, idx: i8, p: u64) {
        if p == 0 {
            return;
        }
        match rd32(p) {
            Some(v) => {
                self.open(idx);
                self.key(b"u32");
                self.w.dec(v as u64);
                self.close();
            }
            None => self.unreadable(idx),
        }
    }
    fn u64_out(&mut self, idx: i8, p: u64) {
        if p == 0 {
            return;
        }
        match rd64(p) {
            Some(v) => {
                self.open(idx);
                self.key(b"u64");
                self.w.hex(v);
                self.close();
            }
            None => self.unreadable(idx),
        }
    }
    #[inline(never)]
    fn bytes(&mut self, idx: i8, p: u64, n: usize) {
        if p == 0 {
            return;
        }
        let mut b = [0u8; 64];
        let want = n.min(64);
        let got = peek(p, &mut b[..want]);
        self.open(idx);
        self.key(b"b");
        put_hex_bytes(self.w, &b[..got]);
        self.key(b"n");
        self.w.dec(n as u64);
        self.close();
    }
    /// A NUL-terminated wide string at `p`.
    #[inline(never)]
    fn wstr(&mut self, idx: i8, p: u64) {
        if p == 0 {
            return;
        }
        let mut s = [0u16; 400];
        match rd_wide(p, &mut s) {
            Some((n, ended)) => {
                self.open(idx);
                self.key(b"w");
                put_wide(self.w, &s[..n], !ended);
                self.close();
            }
            None => self.unreadable(idx),
        }
    }
    /// A wide string of `count` units at `p`.
    #[inline(never)]
    fn wstr_n(&mut self, idx: i8, p: u64, count: u64) {
        if p == 0 || count == 0 {
            return;
        }
        let mut s = [0u16; 400];
        let n = rd_wide_n(p, count.min(400) as usize, &mut s);
        if n == 0 {
            return self.unreadable(idx);
        }
        // A counted string may include its NUL: drop trailing NULs.
        let mut e = n;
        while e > 0 && s[e - 1] == 0 {
            e -= 1;
        }
        self.open(idx);
        self.key(b"w");
        put_wide(self.w, &s[..e], count > 400);
        self.close();
    }
    #[inline(never)]
    fn astr(&mut self, idx: i8, p: u64) {
        if p == 0 {
            return;
        }
        let mut s = [0u8; 400];
        match rd_ansi(p, &mut s) {
            Some((n, ended)) => {
                self.open(idx);
                self.key(b"a");
                put_ansi(self.w, &s[..n], !ended);
                self.close();
            }
            None => self.unreadable(idx),
        }
    }
    #[inline(never)]
    fn astr_n(&mut self, idx: i8, p: u64, count: u64) {
        if p == 0 || count == 0 {
            return;
        }
        let mut s = [0u8; 400];
        let want = count.min(400) as usize;
        let got = peek(p, &mut s[..want]);
        let mut e = got;
        while e > 0 && s[e - 1] == 0 {
            e -= 1;
        }
        self.open(idx);
        self.key(b"a");
        put_ansi(self.w, &s[..e], count > 400);
        self.close();
    }

    /// `UNICODE_STRING` at `p`: `{"us":"text","len":n,"max":m,"bc":<class of Buffer>}`.
    #[inline(never)]
    fn us(&mut self, idx: i8, p: u64) {
        if p == 0 {
            return;
        }
        let (Some(len), Some(max), Some(buf)) = (rd16(p), rd16(p + 2), rd64(p + 8)) else {
            return self.unreadable(idx);
        };
        self.open(idx);
        self.key(b"us");
        self.us_body(len, buf);
        self.key(b"len");
        self.w.dec(len as u64);
        self.key(b"max");
        self.w.dec(max as u64);
        self.key(b"bc");
        pclass(self.w, buf);
        self.close();
    }
    /// The string of a `UNICODE_STRING` (`len` bytes at `buf`), as a JSON string.
    #[inline(never)]
    fn us_body(&mut self, len: u16, buf: u64) {
        let mut s = [0u16; 400];
        let n = if buf == 0 { 0 } else { rd_wide_n(buf, len as usize / 2, &mut s) };
        put_wide(self.w, &s[..n], len as usize / 2 > 400);
    }
    /// `OBJECT_ATTRIBUTES` at `p`.
    #[inline(never)]
    fn oa(&mut self, idx: i8, p: u64) {
        if p == 0 {
            return;
        }
        let (Some(length), Some(root), Some(name), Some(attr)) = (rd32(p), rd64(p + 8), rd64(p + 16), rd32(p + 24))
        else {
            return self.unreadable(idx);
        };
        self.open(idx);
        self.key(b"oa");
        self.w.bytes(b"{\"len\":");
        self.w.dec(length as u64);
        self.w.bytes(b",\"root\":");
        hclass(self.w, root);
        self.w.bytes(b",\"attr\":");
        self.w.hex(attr as u64);
        self.w.bytes(b",\"name\":");
        if name == 0 {
            self.w.bytes(b"null");
        } else if let (Some(l), Some(b)) = (rd16(name), rd64(name + 8)) {
            self.us_body(l, b);
        } else {
            self.w.bytes(b"\"<unreadable>\"");
        }
        self.w.byte(b'}');
        self.close();
    }

    /// Registry data of a type: `"type":T,"dlen":L,"data":...` (no braces).
    #[inline(never)]
    fn regdata(&mut self, ty: u32, p: u64, len: u32) {
        self.w.bytes(b"\"type\":");
        self.w.dec(ty as u64);
        self.w.bytes(b",\"dlen\":");
        self.w.dec(len as u64);
        self.w.bytes(b",\"data\":");
        match ty {
            // REG_SZ, REG_EXPAND_SZ (winnt.h)
            1 | 2 => {
                let mut s = [0u16; 400];
                let n = rd_wide_n(p, len as usize / 2, &mut s);
                let mut e = n;
                while e > 0 && s[e - 1] == 0 {
                    e -= 1;
                }
                put_wide(self.w, &s[..e], len as usize / 2 > 400);
            }
            // REG_DWORD
            4 if len == 4 => match rd32(p) {
                Some(v) => self.w.dec(v as u64),
                None => self.w.bytes(b"null"),
            },
            // REG_QWORD
            11 if len == 8 => match rd64(p) {
                Some(v) => self.w.hex(v),
                None => self.w.bytes(b"null"),
            },
            _ => {
                let mut b = [0u8; 48];
                let got = peek(p, &mut b[..(len as usize).min(48)]);
                put_hex_bytes(self.w, &b[..got]);
            }
        }
    }
    /// A `KEY_VALUE_*_INFORMATION` buffer of the class `class` that `NtQueryValueKey` filled.
    #[inline(never)]
    fn keyvalue(&mut self, idx: i8, buf: u64, class: u32, avail: u32) {
        if buf == 0 {
            return;
        }
        let (Some(ty), Some(w2)) = (rd32(buf + 4), rd32(buf + 8)) else {
            return self.unreadable(idx);
        };
        self.open(idx);
        self.key(b"kv");
        self.w.bytes(b"{\"class\":");
        self.w.dec(class as u64);
        self.w.byte(b',');
        match class {
            // KeyValueBasicInformation: TitleIndex, Type, NameLength, Name
            0 => {
                self.w.bytes(b"\"type\":");
                self.w.dec(ty as u64);
                self.w.bytes(b",\"namelen\":");
                self.w.dec(w2 as u64);
            }
            // KeyValueFullInformation: TitleIndex, Type, DataOffset, DataLength, NameLength, Name
            1 => {
                let dlen = rd32(buf + 12).unwrap_or(0);
                if w2 as u64 + dlen as u64 <= avail as u64 + 64 {
                    self.regdata(ty, buf + w2 as u64, dlen);
                } else {
                    self.w.bytes(b"\"type\":");
                    self.w.dec(ty as u64);
                    self.w.bytes(b",\"dlen\":");
                    self.w.dec(dlen as u64);
                    self.w.bytes(b",\"data\":null");
                }
            }
            // KeyValuePartialInformation: TitleIndex, Type, DataLength, Data
            2 => self.regdata(ty, buf + 12, w2),
            _ => self.w.bytes(b"\"unhandled\":1"),
        }
        self.w.byte(b'}');
        self.close();
    }

    /// `SYSTEM_INFO` at `p`.
    #[inline(never)]
    fn sysinfo(&mut self, idx: i8, p: u64) {
        let mut b = [0u8; 48];
        if peek(p, &mut b) != 48 {
            return self.unreadable(idx);
        }
        let u16at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as u64;
        let u32at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as u64;
        let u64at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap_or([0; 8]));
        self.open(idx);
        self.key(b"sysinfo");
        self.w.bytes(b"{\"arch\":");
        self.w.dec(u16at(0));
        self.w.bytes(b",\"page\":");
        self.w.dec(u32at(4));
        self.w.bytes(b",\"minaddr\":");
        self.w.hex(u64at(8));
        self.w.bytes(b",\"maxaddr\":");
        self.w.hex(u64at(16));
        self.w.bytes(b",\"mask\":");
        self.w.hex(u64at(24));
        self.w.bytes(b",\"cpus\":");
        self.w.dec(u32at(32));
        self.w.bytes(b",\"type\":");
        self.w.dec(u32at(36));
        self.w.bytes(b",\"granularity\":");
        self.w.dec(u32at(40));
        self.w.bytes(b",\"level\":");
        self.w.dec(u16at(44));
        self.w.bytes(b",\"revision\":");
        self.w.dec(u16at(46));
        self.w.byte(b'}');
        self.close();
    }
    /// `OSVERSIONINFOEXW` (or the shorter `OSVERSIONINFOW`) at `p`.
    #[inline(never)]
    fn osvi(&mut self, idx: i8, p: u64) {
        let mut b = [0u8; 284];
        let got = peek(p, &mut b);
        if got < 20 {
            return self.unreadable(idx);
        }
        let u32at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as u64;
        self.open(idx);
        self.key(b"osvi");
        self.w.bytes(b"{\"size\":");
        self.w.dec(u32at(0));
        self.w.bytes(b",\"major\":");
        self.w.dec(u32at(4));
        self.w.bytes(b",\"minor\":");
        self.w.dec(u32at(8));
        self.w.bytes(b",\"build\":");
        self.w.dec(u32at(12));
        self.w.bytes(b",\"platform\":");
        self.w.dec(u32at(16));
        if got >= 276 {
            let mut s = [0u16; 128];
            let mut n = 0;
            while n < 128 {
                let u = u16::from_le_bytes([b[20 + 2 * n], b[21 + 2 * n]]);
                if u == 0 {
                    break;
                }
                s[n] = u;
                n += 1;
            }
            self.w.bytes(b",\"csd\":");
            put_wide(self.w, &s[..n], false);
        }
        if got >= 284 {
            let u16at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as u64;
            self.w.bytes(b",\"spmajor\":");
            self.w.dec(u16at(276));
            self.w.bytes(b",\"spminor\":");
            self.w.dec(u16at(278));
            self.w.bytes(b",\"suite\":");
            self.w.dec(u16at(280));
            self.w.bytes(b",\"product\":");
            self.w.dec(b[282] as u64);
        }
        self.w.byte(b'}');
        self.close();
    }
    /// `STARTUPINFOW` at `p`.
    #[inline(never)]
    fn startupinfo(&mut self, idx: i8, p: u64) {
        let mut b = [0u8; 104];
        if peek(p, &mut b) != 104 {
            return self.unreadable(idx);
        }
        let u32at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as u64;
        let u16at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as u64;
        let u64at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap_or([0; 8]));
        self.open(idx);
        self.key(b"startupinfo");
        self.w.bytes(b"{\"cb\":");
        self.w.dec(u32at(0));
        self.w.bytes(b",\"flags\":");
        self.w.hex(u32at(60));
        self.w.bytes(b",\"show\":");
        self.w.dec(u16at(64));
        self.w.bytes(b",\"res2\":");
        self.w.dec(u16at(66));
        for (k, off) in [(&b",\"desktop\":"[..], 16usize), (&b",\"title\":"[..], 24)] {
            self.w.bytes(k);
            let ptr = u64at(off);
            let mut s = [0u16; 200];
            match if ptr == 0 { None } else { rd_wide(ptr, &mut s) } {
                Some((n, ended)) => put_wide(self.w, &s[..n], !ended),
                None => self.w.bytes(b"null"),
            }
        }
        for (k, off) in [(&b",\"stdin\":"[..], 80usize), (&b",\"stdout\":"[..], 88), (&b",\"stderr\":"[..], 96)] {
            self.w.bytes(k);
            hclass(self.w, u64at(off));
        }
        self.w.byte(b'}');
        self.close();
    }
    /// `MEMORY_BASIC_INFORMATION` at `p`.
    #[inline(never)]
    fn mbi(&mut self, idx: i8, p: u64) {
        let mut b = [0u8; 48];
        if peek(p, &mut b) != 48 {
            return self.unreadable(idx);
        }
        let u32at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as u64;
        let u64at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap_or([0; 8]));
        self.open(idx);
        self.key(b"mbi");
        self.w.bytes(b"{\"base\":");
        pclass(self.w, u64at(0));
        self.w.bytes(b",\"allocbase\":");
        pclass(self.w, u64at(8));
        self.w.bytes(b",\"allocprot\":");
        self.w.hex(u32at(16));
        self.w.bytes(b",\"size\":");
        self.w.hex(u64at(24));
        self.w.bytes(b",\"state\":");
        self.w.hex(u32at(32));
        self.w.bytes(b",\"protect\":");
        self.w.hex(u32at(36));
        self.w.bytes(b",\"type\":");
        self.w.hex(u32at(40));
        self.w.byte(b'}');
        self.close();
    }
    /// `RTL_CRITICAL_SECTION` at `p`, after `InitializeCriticalSectionEx`.
    #[inline(never)]
    fn crit(&mut self, idx: i8, p: u64) {
        let (Some(dbg), Some(lc), Some(rc), Some(own), Some(sem), Some(spin)) =
            (rd64(p), rd32(p + 8), rd32(p + 12), rd64(p + 16), rd64(p + 24), rd64(p + 32))
        else {
            return self.unreadable(idx);
        };
        self.open(idx);
        self.key(b"cs");
        self.w.bytes(b"{\"dbg\":");
        pclass(self.w, dbg);
        self.w.bytes(b",\"lock\":");
        sdec(self.w, lc as i32 as i64);
        self.w.bytes(b",\"rec\":");
        sdec(self.w, rc as i32 as i64);
        self.w.bytes(b",\"owner\":");
        pclass(self.w, own);
        self.w.bytes(b",\"sem\":");
        pclass(self.w, sem);
        self.w.bytes(b",\"spin\":");
        self.w.dec(spin);
        self.w.byte(b'}');
        self.close();
    }
    /// A module handle result: the module's base name.
    #[inline(never)]
    fn module_ret(&mut self, idx: i8, v: u64) {
        self.open(idx);
        if v == 0 {
            self.w.bytes(b",\"module\":null");
        } else if let Some((_, slot)) = module_of(v) {
            self.w.bytes(b",\"module\":\"");
            put_mod_name(self.w, slot);
            self.w.byte(b'"');
        } else {
            self.w.bytes(b",\"module\":\"?\"");
        }
        self.close();
    }
    /// The environment block `p` (`GetEnvironmentStringsW`): size, count and variable names.
    #[inline(never)]
    fn envblock(&mut self, idx: i8, p: u64) {
        if p == 0 {
            return;
        }
        let mut chars = 0u64;
        let mut vars = 0u64;
        let mut names = [[0u16; 24]; 32];
        let mut name_len = [0usize; 32];
        let mut at = p;
        loop {
            let mut s = [0u16; 400];
            let Some((n, ended)) = rd_wide(at, &mut s) else { break };
            if n == 0 && ended {
                chars += 1;
                break;
            }
            if (vars as usize) < 32 {
                // the name is what precedes '=' (a leading '=' belongs to the name: "=C:")
                let eq = s[1.min(n)..n].iter().position(|&u| u == b'=' as u16).map(|e| e + 1).unwrap_or(n);
                let l = eq.min(24);
                names[vars as usize][..l].copy_from_slice(&s[..l]);
                name_len[vars as usize] = l;
            }
            vars += 1;
            chars += n as u64 + 1;
            if !ended || chars > 32768 {
                break;
            }
            at += (n as u64 + 1) * 2;
        }
        self.open(idx);
        self.key(b"env");
        self.w.bytes(b"{\"chars\":");
        self.w.dec(chars);
        self.w.bytes(b",\"vars\":");
        self.w.dec(vars);
        self.w.bytes(b",\"names\":[");
        for i in 0..(vars as usize).min(32) {
            if i > 0 {
                self.w.byte(b',');
            }
            put_wide(self.w, &names[i][..name_len[i]], false);
        }
        self.w.bytes(b"]}");
        self.close();
    }
    /// The result of `VerQueryValueW(block, subblock, &ptr, &len)`.
    #[inline(never)]
    fn verquery(&mut self, subblock: u64, pp: u64, lp: u64) {
        let (Some(ptr), Some(len)) = (rd64(pp), rd32(lp)) else { return };
        self.cls(2, ptr);
        self.u32_out(3, lp);
        let mut s = [0u16; 64];
        let sub = rd_wide(subblock, &mut s).map(|(n, _)| n).unwrap_or(0);
        let is = |lit: &[u8]| sub == lit.len() && lit.iter().zip(s.iter()).all(|(&a, &b)| a as u16 == b);
        if is(b"\\") && len >= 52 {
            // VS_FIXEDFILEINFO: signature, struc version, file version ms/ls, product version ms/ls, ...
            let mut b = [0u8; 52];
            if peek(ptr, &mut b) == 52 {
                self.open(2);
                self.key(b"fixedfileinfo");
                self.w.byte(b'[');
                for i in 0..13 {
                    if i > 0 {
                        self.w.byte(b',');
                    }
                    self.w.hex(u32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]) as u64);
                }
                self.w.byte(b']');
                self.close();
            }
        } else if len > 0 && len < 200 && ptr != 0 {
            self.bytes(2, ptr, len as usize);
        }
    }
}

/// Write the `"out"` entries for a call of `name` that returned `ret`; `a` are its first twelve
/// argument values (raw). `ok` is false when nothing should be read (never for now).
pub fn dump(d: &mut D, name: &[u8], a: &[u64; 12], ret: u64) {
    let r32 = ret as u32;
    match name {
        // ----- modules, paths, environment, command line
        b"GetModuleFileNameW" => {
            if r32 > 0 && (r32 as u64) <= a[2] {
                d.wstr_n(1, a[1], r32 as u64);
            }
        }
        b"GetModuleFileNameA" => {
            if r32 > 0 && (r32 as u64) <= a[2] {
                d.astr_n(1, a[1], r32 as u64);
            }
        }
        b"GetModuleHandleW" | b"GetModuleHandleA" | b"LoadLibraryW" | b"LoadLibraryA" | b"LoadLibraryExW"
        | b"LoadLibraryExA" => d.module_ret(RET, ret),
        b"GetModuleHandleExW" => {
            if r32 != 0 && a[2] != 0 {
                if let Some(h) = rd64(a[2]) {
                    d.module_ret(2, h);
                }
            }
        }
        b"GetProcAddress" => d.cls(RET, ret),
        b"GetEnvironmentVariableW" => {
            if r32 > 0 && (r32 as u64) < a[2] {
                d.wstr_n(1, a[1], r32 as u64);
            }
        }
        b"GetEnvironmentVariableA" => {
            if r32 > 0 && (r32 as u64) < a[2] {
                d.astr_n(1, a[1], r32 as u64);
            }
        }
        b"ExpandEnvironmentStringsW" => {
            if r32 > 0 && (r32 as u64) <= a[2] {
                d.wstr(1, a[1]);
            }
        }
        b"GetCommandLineW" => d.wstr(RET, ret),
        b"GetCommandLineA" => d.astr(RET, ret),
        b"GetEnvironmentStringsW" => d.envblock(RET, ret),
        b"GetSystemDirectoryW"
        | b"GetWindowsDirectoryW"
        | b"GetSystemWindowsDirectoryW"
        | b"GetSystemWow64DirectoryW" => {
            if r32 > 0 && (r32 as u64) < a[1] {
                d.wstr_n(0, a[0], r32 as u64);
            }
        }
        b"GetCurrentDirectoryW" | b"GetTempPathW" => {
            if r32 > 0 && (r32 as u64) < a[0] {
                d.wstr_n(1, a[1], r32 as u64);
            }
        }
        b"GetFullPathNameW" => {
            if r32 > 0 && (r32 as u64) < a[1] {
                d.wstr_n(2, a[2], r32 as u64);
                d.ptr_out(3, a[3], false);
            }
        }
        b"GetStartupInfoW" => d.startupinfo(0, a[0]),
        b"GetStdHandle" | b"GetCurrentProcess" | b"GetCurrentThread" | b"CreateMutexW" | b"CreateMutexExW"
        | b"CreateEventW" | b"CreateEventExW" | b"CreateSemaphoreW" | b"OpenProcess" => d.hcls(RET, ret),
        // ----- system and version
        b"GetNativeSystemInfo" | b"GetSystemInfo" => d.sysinfo(0, a[0]),
        b"VerifyVersionInfoW" | b"RtlGetVersion" | b"GetVersionExW" => d.osvi(0, a[0]),
        b"IsWow64Process" => d.u32_out(1, a[1]),
        b"GetCPInfo" => {
            if r32 != 0 {
                d.bytes(1, a[1], 18);
            }
        }
        b"GetFileVersionInfoSizeW" => d.u32_out(1, a[1]),
        b"GetFileVersionInfoSizeExW" => d.u32_out(2, a[2]),
        b"GetFileVersionInfoW" => {
            if r32 != 0 {
                d.bytes(3, a[3], (a[2] as usize).min(32));
            }
        }
        b"GetFileVersionInfoExW" => {
            if r32 != 0 {
                d.bytes(4, a[4], (a[3] as usize).min(32));
            }
        }
        b"VerQueryValueW" => {
            if r32 != 0 {
                d.verquery(a[1], a[2], a[3]);
            }
        }
        b"QueryPerformanceFrequency" | b"QueryPerformanceCounter" => d.u64_out(0, a[0]),
        // ----- memory, heap, TLS/FLS
        b"VirtualAlloc" | b"HeapAlloc" | b"HeapReAlloc" | b"GetProcessHeap" | b"HeapCreate" | b"MapViewOfFile"
        | b"MapViewOfFileEx" | b"FlsGetValue" | b"TlsGetValue" => d.cls(RET, ret),
        b"VirtualProtect" => {
            d.cls(0, a[0]);
            if r32 != 0 {
                d.u32_out(3, a[3]);
            }
        }
        b"VirtualQuery" => {
            d.cls(0, a[0]);
            if ret != 0 {
                d.mbi(1, a[1]);
            }
        }
        b"VirtualFree" => d.cls(0, a[0]),
        b"FlsSetValue" | b"TlsSetValue" => d.cls(1, a[1]),
        b"InitializeCriticalSectionEx" | b"InitializeCriticalSection" | b"InitializeCriticalSectionAndSpinCount" => {
            d.crit(0, a[0])
        }
        b"InitializeSListHead" => d.bytes(0, a[0], 16),
        b"SetUnhandledExceptionFilter" => {
            d.cls(0, a[0]);
            d.cls(RET, ret);
        }
        // ----- process memory
        b"ReadProcessMemory" => {
            d.hcls(0, a[0]);
            d.cls(1, a[1]);
            if r32 != 0 {
                d.u64_out(4, a[4]);
                let n = if a[4] != 0 { rd64(a[4]).unwrap_or(a[3]) } else { a[3] };
                d.bytes(2, a[2], n.min(64) as usize);
            }
        }
        b"WriteProcessMemory" => {
            d.hcls(0, a[0]);
            d.cls(1, a[1]);
            d.bytes(2, a[2], a[3].min(64) as usize);
            if r32 != 0 {
                d.u64_out(4, a[4]);
            }
        }
        b"DuplicateHandle" => {
            d.hcls(0, a[0]);
            d.hcls(1, a[1]);
            d.hcls(2, a[2]);
            if r32 != 0 {
                d.ptr_out(3, a[3], true);
            }
        }
        // ----- strings
        b"MultiByteToWideChar" => {
            if r32 > 0 && a[4] != 0 {
                d.wstr_n(4, a[4], (r32 as u64).min(64));
            }
        }
        b"WideCharToMultiByte" => {
            if r32 > 0 && a[4] != 0 {
                d.astr_n(4, a[4], (r32 as u64).min(64));
            }
        }
        // ----- ntdll: strings and registry
        b"RtlInitUnicodeString" => d.us(0, a[0]),
        b"RtlFreeUnicodeString" => d.us(0, a[0]),
        b"RtlFormatCurrentUserKeyPath" => {
            if r32 == 0 {
                d.us(0, a[0]);
            }
        }
        b"NtOpenKeyEx" | b"NtOpenKey" => {
            d.oa(2, a[2]);
            if r32 == 0 {
                d.ptr_out(0, a[0], true);
            }
        }
        b"NtCreateKey" => {
            d.oa(2, a[2]);
            d.us(4, a[4]);
            if r32 == 0 {
                d.ptr_out(0, a[0], true);
                d.u32_out(6, a[6]);
            }
        }
        b"NtQueryValueKey" => {
            d.us(1, a[1]);
            d.u32_out(5, a[5]);
            if r32 == 0 || r32 == 0x8000_0005 {
                let got = if a[5] != 0 { rd32(a[5]).unwrap_or(0) } else { 0 };
                let avail = (a[4] as u32).min(got);
                if r32 == 0 {
                    d.keyvalue(3, a[3], a[2] as u32, avail);
                }
            }
        }
        b"NtSetValueKey" => {
            d.us(1, a[1]);
            d.open(4);
            d.key(b"kv");
            d.w.byte(b'{');
            d.regdata(a[3] as u32, a[4], a[5] as u32);
            d.w.byte(b'}');
            d.close();
        }
        b"NtDeleteValueKey" => d.us(1, a[1]),
        b"NtQueryInformationProcess" | b"NtQuerySystemInformation" | b"NtQueryInformationThread" => {
            let (len_arg, buf_arg) = if name == b"NtQuerySystemInformation" { (3, 1) } else { (4, 2) };
            d.u32_out(len_arg as i8, a[len_arg]);
            if r32 == 0 {
                let n = if a[len_arg] != 0 { rd32(a[len_arg]).unwrap_or(0) } else { 0 };
                d.bytes(buf_arg as i8, a[buf_arg], (n as usize).min(32));
            }
        }
        _ => {}
    }
}

/// Whether [`dump`] has anything to say about `name` (so the logger spares the work for the rest).
pub fn has_rule(name: &[u8]) -> bool {
    matches!(
        name,
        b"GetModuleFileNameW"
            | b"GetModuleFileNameA"
            | b"GetModuleHandleW"
            | b"GetModuleHandleA"
            | b"LoadLibraryW"
            | b"LoadLibraryA"
            | b"LoadLibraryExW"
            | b"LoadLibraryExA"
            | b"GetModuleHandleExW"
            | b"GetProcAddress"
            | b"GetEnvironmentVariableW"
            | b"GetEnvironmentVariableA"
            | b"ExpandEnvironmentStringsW"
            | b"GetCommandLineW"
            | b"GetCommandLineA"
            | b"GetEnvironmentStringsW"
            | b"GetSystemDirectoryW"
            | b"GetWindowsDirectoryW"
            | b"GetSystemWindowsDirectoryW"
            | b"GetSystemWow64DirectoryW"
            | b"GetCurrentDirectoryW"
            | b"GetTempPathW"
            | b"GetFullPathNameW"
            | b"GetStartupInfoW"
            | b"GetStdHandle"
            | b"GetCurrentProcess"
            | b"GetCurrentThread"
            | b"CreateMutexW"
            | b"CreateMutexExW"
            | b"CreateEventW"
            | b"CreateEventExW"
            | b"CreateSemaphoreW"
            | b"OpenProcess"
            | b"GetNativeSystemInfo"
            | b"GetSystemInfo"
            | b"VerifyVersionInfoW"
            | b"RtlGetVersion"
            | b"GetVersionExW"
            | b"IsWow64Process"
            | b"GetCPInfo"
            | b"GetFileVersionInfoSizeW"
            | b"GetFileVersionInfoSizeExW"
            | b"GetFileVersionInfoW"
            | b"GetFileVersionInfoExW"
            | b"VerQueryValueW"
            | b"QueryPerformanceFrequency"
            | b"QueryPerformanceCounter"
            | b"VirtualAlloc"
            | b"HeapAlloc"
            | b"HeapReAlloc"
            | b"GetProcessHeap"
            | b"HeapCreate"
            | b"MapViewOfFile"
            | b"MapViewOfFileEx"
            | b"FlsGetValue"
            | b"TlsGetValue"
            | b"VirtualProtect"
            | b"VirtualQuery"
            | b"VirtualFree"
            | b"FlsSetValue"
            | b"TlsSetValue"
            | b"InitializeCriticalSectionEx"
            | b"InitializeCriticalSection"
            | b"InitializeCriticalSectionAndSpinCount"
            | b"InitializeSListHead"
            | b"SetUnhandledExceptionFilter"
            | b"ReadProcessMemory"
            | b"WriteProcessMemory"
            | b"DuplicateHandle"
            | b"MultiByteToWideChar"
            | b"WideCharToMultiByte"
            | b"RtlInitUnicodeString"
            | b"RtlFreeUnicodeString"
            | b"RtlFormatCurrentUserKeyPath"
            | b"NtOpenKeyEx"
            | b"NtOpenKey"
            | b"NtCreateKey"
            | b"NtQueryValueKey"
            | b"NtSetValueKey"
            | b"NtDeleteValueKey"
            | b"NtQueryInformationProcess"
            | b"NtQuerySystemInformation"
            | b"NtQueryInformationThread"
    )
}
