//! Counted strings (`ntdef.h`): `UNICODE_STRING` and `ANSI_STRING`
//! (`STRING`), and the `Rtl*` functions over them as Learn's WDK reference
//! pages describe them.
//!
//! `Length` and `MaximumLength` are in **bytes** and the buffer need not be
//! NUL-terminated. Functions that write a destination string never write
//! past its `MaximumLength`; where Learn says nothing about a terminating
//! NUL we add one only when it fits beyond `Length` (to be confirmed by the
//! spec agent, ADR 0071). Comparisons ignore case by the object manager's
//! upcase rule ([`poly_ntobj::name::upcase`]), so names compare the same in
//! every part of the NT personality. ANSI strings are in Poly's ANSI code
//! page, Windows-1252 ([`crate::codepage`]).
//!
//! Every function is `unsafe`: the strings' buffers are the caller's
//! memory, and `ntdll` passes its callers' pointers straight through. The
//! safety contract for all of them: each string pointer is null or points
//! at a valid string structure whose `Buffer` is valid for `MaximumLength`
//! bytes (for reading: `Length` bytes), and destination buffers do not
//! overlap sources unless stated.

use crate::codepage;
use core::ptr;
use poly_ntobj::NtStatus;
use poly_ntobj::name::upcase;

/// `STATUS_BUFFER_OVERFLOW` (`ntstatus.h`).
pub const STATUS_BUFFER_OVERFLOW: NtStatus = NtStatus(0x8000_0005);
/// `STATUS_BUFFER_TOO_SMALL`.
pub const STATUS_BUFFER_TOO_SMALL: NtStatus = NtStatus(0xC000_0023);
/// `STATUS_NO_MEMORY`.
pub const STATUS_NO_MEMORY: NtStatus = NtStatus(0xC000_0017);
/// `STATUS_INVALID_PARAMETER`.
pub const STATUS_INVALID_PARAMETER: NtStatus = NtStatus(0xC000_000D);
/// `STATUS_INVALID_PARAMETER_2`.
pub const STATUS_INVALID_PARAMETER_2: NtStatus = NtStatus(0xC000_00F0);
/// `STATUS_NAME_TOO_LONG`.
pub const STATUS_NAME_TOO_LONG: NtStatus = NtStatus(0xC000_0106);
/// `STATUS_SUCCESS`.
pub const STATUS_SUCCESS: NtStatus = NtStatus(0);

/// The longest counted string, in bytes (`UNICODE_STRING_MAX_BYTES` in
/// `ntdef.h` is 65534).
pub const MAX_BYTES: usize = 0xFFFE;

/// `UNICODE_STRING` (`ntdef.h`): 16 bytes on x64, `Buffer` at offset 8.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct UnicodeString {
    pub length: u16,
    pub maximum_length: u16,
    pub buffer: *mut u16,
}

/// `STRING` / `ANSI_STRING` (`ntdef.h`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AnsiString {
    pub length: u16,
    pub maximum_length: u16,
    pub buffer: *mut u8,
}

impl UnicodeString {
    pub const EMPTY: UnicodeString = UnicodeString { length: 0, maximum_length: 0, buffer: ptr::null_mut() };

    /// The string's code units.
    ///
    /// # Safety
    /// `buffer` is valid for `length` bytes (or `length` is 0).
    pub unsafe fn units(&self) -> &[u16] {
        if self.buffer.is_null() || self.length < 2 {
            return &[];
        }
        // SAFETY: the caller's contract: `buffer` holds `length` bytes.
        unsafe { core::slice::from_raw_parts(self.buffer, usize::from(self.length) / 2) }
    }
}

impl AnsiString {
    pub const EMPTY: AnsiString = AnsiString { length: 0, maximum_length: 0, buffer: ptr::null_mut() };

    /// The string's bytes.
    ///
    /// # Safety
    /// `buffer` is valid for `length` bytes (or `length` is 0).
    pub unsafe fn bytes(&self) -> &[u8] {
        if self.buffer.is_null() || self.length == 0 {
            return &[];
        }
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(self.buffer, usize::from(self.length)) }
    }
}

/// Where the functions that allocate a destination (`Allocate` = TRUE) get
/// their buffers: `ntdll` uses the process heap, so `RtlFreeUnicodeString`
/// can free them.
pub trait Allocator {
    /// `bytes` of memory, or null.
    fn alloc(&self, bytes: usize) -> *mut u8;
    /// Free what [`alloc`](Self::alloc) returned.
    fn free(&self, p: *mut u8);
}

/// Length of a NUL-terminated wide string, in code units, looking at most
/// `limit` units.
///
/// # Safety
/// `s` points at a NUL-terminated string or at least `limit` readable units.
pub unsafe fn wcslen(s: *const u16, limit: usize) -> usize {
    let mut n = 0;
    // SAFETY: the caller's contract; we stop at the NUL or `limit`.
    while n < limit && unsafe { *s.add(n) } != 0 {
        n += 1;
    }
    n
}

/// Length of a NUL-terminated byte string, looking at most `limit` bytes.
///
/// # Safety
/// As [`wcslen`].
pub unsafe fn strlen(s: *const u8, limit: usize) -> usize {
    let mut n = 0;
    // SAFETY: the caller's contract.
    while n < limit && unsafe { *s.add(n) } != 0 {
        n += 1;
    }
    n
}

/// `RtlInitUnicodeString`: `dest` describes the NUL-terminated `source`
/// (null: an empty string with a null buffer). A source longer than the
/// longest counted string is cut to [`MAX_BYTES`] - 2 bytes (to be
/// confirmed).
///
/// # Safety
/// `dest` is writable; `source` is null or NUL-terminated.
pub unsafe fn init_unicode_string(dest: *mut UnicodeString, source: *const u16) {
    let s = if source.is_null() {
        UnicodeString::EMPTY
    } else {
        // SAFETY: the caller's contract.
        let n = unsafe { wcslen(source, MAX_BYTES / 2) } * 2;
        let len = n.min(MAX_BYTES - 2);
        UnicodeString { length: len as u16, maximum_length: (len + 2) as u16, buffer: source as *mut u16 }
    };
    // SAFETY: `dest` is writable.
    unsafe { dest.write(s) };
}

/// `RtlInitUnicodeStringEx`: like [`init_unicode_string`], but a source
/// too long for a counted string is `STATUS_NAME_TOO_LONG` (and `dest` is
/// left alone).
///
/// # Safety
/// As [`init_unicode_string`].
pub unsafe fn init_unicode_string_ex(dest: *mut UnicodeString, source: *const u16) -> NtStatus {
    if !source.is_null() {
        // SAFETY: the caller's contract.
        let n = unsafe { wcslen(source, MAX_BYTES / 2) } * 2;
        if n > MAX_BYTES - 2 {
            return STATUS_NAME_TOO_LONG;
        }
    }
    // SAFETY: the caller's contract.
    unsafe { init_unicode_string(dest, source) };
    STATUS_SUCCESS
}

/// `RtlInitAnsiString` / `RtlInitString`.
///
/// # Safety
/// As [`init_unicode_string`].
pub unsafe fn init_ansi_string(dest: *mut AnsiString, source: *const u8) {
    let s = if source.is_null() {
        AnsiString::EMPTY
    } else {
        // SAFETY: the caller's contract.
        let n = unsafe { strlen(source, MAX_BYTES) }.min(MAX_BYTES - 1);
        AnsiString { length: n as u16, maximum_length: (n + 1) as u16, buffer: source as *mut u8 }
    };
    // SAFETY: `dest` is writable.
    unsafe { dest.write(s) };
}

/// Write a NUL after `length` bytes of `dest` if it fits.
///
/// # Safety
/// `dest`'s buffer is valid for `maximum_length` bytes.
unsafe fn terminate_unicode(dest: &UnicodeString) {
    if !dest.buffer.is_null() && usize::from(dest.length) + 2 <= usize::from(dest.maximum_length) {
        // SAFETY: inside `maximum_length`, checked above.
        unsafe { dest.buffer.add(usize::from(dest.length) / 2).write(0) };
    }
}

/// `RtlCopyUnicodeString`: copy as much of `source` as `dest`'s
/// `MaximumLength` holds (Learn: "whichever is smaller"); a null source
/// makes `dest` empty.
///
/// # Safety
/// The module's contract; `dest` is writable.
pub unsafe fn copy_unicode_string(dest: *mut UnicodeString, source: *const UnicodeString) {
    // SAFETY: the caller's contract.
    let d = unsafe { &mut *dest };
    if source.is_null() {
        d.length = 0;
        return;
    }
    // SAFETY: the caller's contract.
    let s = unsafe { &*source };
    let n = usize::from(s.length.min(d.maximum_length)) & !1;
    if n > 0 {
        // SAFETY: both buffers hold at least `n` bytes; `copy` allows overlap.
        unsafe { ptr::copy(s.buffer as *const u8, d.buffer as *mut u8, n) };
    }
    d.length = n as u16;
    // SAFETY: the caller's contract.
    unsafe { terminate_unicode(d) };
}

/// Compare two code-unit sequences, optionally ignoring case: the first
/// differing unit decides, else the shorter string is less.
pub fn compare_units(a: &[u16], b: &[u16], ignore_case: bool) -> i32 {
    for (&x, &y) in a.iter().zip(b) {
        let (x, y) = if ignore_case { (upcase(x), upcase(y)) } else { (x, y) };
        if x != y {
            return i32::from(x) - i32::from(y);
        }
    }
    (a.len() as i32) - (b.len() as i32)
}

/// `RtlCompareUnicodeString`: < 0, 0 or > 0.
///
/// # Safety
/// The module's contract.
pub unsafe fn compare_unicode_string(a: *const UnicodeString, b: *const UnicodeString, ignore_case: bool) -> i32 {
    // SAFETY: the caller's contract.
    unsafe { compare_units((*a).units(), (*b).units(), ignore_case) }
}

/// `RtlEqualUnicodeString`.
///
/// # Safety
/// The module's contract.
pub unsafe fn equal_unicode_string(a: *const UnicodeString, b: *const UnicodeString, ignore_case: bool) -> bool {
    // SAFETY: the caller's contract.
    let (a, b) = unsafe { ((*a).units(), (*b).units()) };
    a.len() == b.len() && compare_units(a, b, ignore_case) == 0
}

/// `RtlPrefixUnicodeString`: whether `prefix` starts `string`.
///
/// # Safety
/// The module's contract.
pub unsafe fn prefix_unicode_string(
    prefix: *const UnicodeString,
    string: *const UnicodeString,
    ignore_case: bool,
) -> bool {
    // SAFETY: the caller's contract.
    let (p, s) = unsafe { ((*prefix).units(), (*string).units()) };
    p.len() <= s.len() && compare_units(p, &s[..p.len()], ignore_case) == 0
}

/// `RtlUpcaseUnicodeChar`.
pub fn upcase_char(c: u16) -> u16 {
    upcase(c)
}

/// `RtlDowncaseUnicodeChar`: the mirror of the upcase rule (a single BMP
/// lowercase mapping, else unchanged; surrogates unchanged).
pub fn downcase_char(c: u16) -> u16 {
    if c < 0x80 {
        return (c as u8).to_ascii_lowercase() as u16;
    }
    let Some(ch) = char::from_u32(u32::from(c)) else { return c };
    let mut low = ch.to_lowercase();
    match (low.next(), low.next()) {
        (Some(l), None) if (l as u32) <= 0xFFFF => l as u16,
        _ => c,
    }
}

/// Prepare `dest` for `bytes` bytes: allocate it, or check it is big
/// enough.
///
/// # Safety
/// The module's contract.
unsafe fn make_room_unicode(
    dest: &mut UnicodeString,
    bytes: usize,
    with_nul: bool,
    allocate: bool,
    a: &dyn Allocator,
) -> Result<(), NtStatus> {
    if bytes > MAX_BYTES {
        return Err(STATUS_INVALID_PARAMETER_2);
    }
    if allocate {
        let size = (bytes + if with_nul { 2 } else { 0 }).clamp(2, MAX_BYTES);
        let p = a.alloc(size);
        if p.is_null() {
            return Err(STATUS_NO_MEMORY);
        }
        dest.buffer = p as *mut u16;
        dest.maximum_length = size as u16;
    } else if bytes > usize::from(dest.maximum_length) {
        return Err(STATUS_BUFFER_OVERFLOW);
    }
    Ok(())
}

/// `RtlUpcaseUnicodeString` / `RtlDowncaseUnicodeString` (`up` chooses):
/// `dest` gets `source` with each unit mapped; `allocate` makes a new
/// buffer (free it with [`free_unicode_string`]), otherwise `dest` must
/// hold `source.Length` bytes (`STATUS_BUFFER_OVERFLOW`). `dest` may be
/// `source`.
///
/// # Safety
/// The module's contract.
pub unsafe fn case_unicode_string(
    dest: *mut UnicodeString,
    source: *const UnicodeString,
    allocate: bool,
    up: bool,
    a: &dyn Allocator,
) -> NtStatus {
    // SAFETY: the caller's contract; copy the source descriptor first, as
    // `dest` may be the same structure.
    let s = unsafe { *source };
    // SAFETY: the caller's contract.
    let d = unsafe { &mut *dest };
    let n = usize::from(s.length) & !1;
    // SAFETY: the caller's contract.
    if let Err(e) = unsafe { make_room_unicode(d, n, false, allocate, a) } {
        return e;
    }
    for i in 0..n / 2 {
        // SAFETY: both buffers hold `n` bytes; one unit at a time, so the
        // same buffer for both is fine.
        unsafe {
            let c = s.buffer.add(i).read();
            d.buffer.add(i).write(if up { upcase(c) } else { downcase_char(c) });
        }
    }
    d.length = n as u16;
    STATUS_SUCCESS
}

/// `RtlAnsiStringToUnicodeString`: code page 1252 to UTF-16. With
/// `allocate`, the new buffer has room for a NUL (`RtlAnsiStringToUnicodeSize`).
///
/// # Safety
/// The module's contract.
pub unsafe fn ansi_to_unicode_string(
    dest: *mut UnicodeString,
    source: *const AnsiString,
    allocate: bool,
    a: &dyn Allocator,
) -> NtStatus {
    // SAFETY: the caller's contract.
    let s = unsafe { *source };
    // SAFETY: the caller's contract.
    let d = unsafe { &mut *dest };
    let n = usize::from(s.length) * 2;
    // SAFETY: the caller's contract.
    if let Err(e) = unsafe { make_room_unicode(d, n, true, allocate, a) } {
        return e;
    }
    // SAFETY: the caller's contract.
    for (i, &b) in unsafe { s.bytes() }.iter().enumerate() {
        // SAFETY: `d` holds `n` bytes.
        unsafe { d.buffer.add(i).write(codepage::to_unicode(b)) };
    }
    d.length = n as u16;
    // SAFETY: the caller's contract.
    unsafe { terminate_unicode(d) };
    STATUS_SUCCESS
}

/// `RtlUnicodeStringToAnsiString`: UTF-16 to code page 1252 (`?` for what
/// it cannot hold). With `allocate` the buffer gets room for a NUL; a
/// destination too small gets as much as fits and `STATUS_BUFFER_OVERFLOW`
/// (to be confirmed).
///
/// # Safety
/// The module's contract.
pub unsafe fn unicode_to_ansi_string(
    dest: *mut AnsiString,
    source: *const UnicodeString,
    allocate: bool,
    a: &dyn Allocator,
) -> NtStatus {
    // SAFETY: the caller's contract.
    let s = unsafe { *source };
    // SAFETY: the caller's contract.
    let d = unsafe { &mut *dest };
    let n = usize::from(s.length) / 2;
    let mut status = STATUS_SUCCESS;
    if allocate {
        let size = (n + 1).min(MAX_BYTES);
        let p = a.alloc(size);
        if p.is_null() {
            return STATUS_NO_MEMORY;
        }
        d.buffer = p;
        d.maximum_length = size as u16;
    }
    let room = usize::from(d.maximum_length);
    let take = if n > room {
        status = STATUS_BUFFER_OVERFLOW;
        room.saturating_sub(1)
    } else {
        n
    };
    // SAFETY: the caller's contract.
    for (i, &c) in unsafe { s.units() }.iter().take(take).enumerate() {
        // SAFETY: `take` <= `maximum_length`.
        unsafe { d.buffer.add(i).write(codepage::from_unicode(c)) };
    }
    d.length = take as u16;
    if !d.buffer.is_null() && take < room {
        // SAFETY: inside `maximum_length`.
        unsafe { d.buffer.add(take).write(0) };
    }
    status
}

/// `RtlAnsiStringToUnicodeSize`: bytes [`ansi_to_unicode_string`] needs
/// with `allocate`, the NUL included.
///
/// # Safety
/// The module's contract.
pub unsafe fn ansi_to_unicode_size(source: *const AnsiString) -> u32 {
    // SAFETY: the caller's contract.
    (u32::from(unsafe { (*source).length }) + 1) * 2
}

/// `RtlFreeUnicodeString`: free a buffer an `allocate` call made and empty
/// the string.
///
/// # Safety
/// The buffer is null or came from `a`.
pub unsafe fn free_unicode_string(s: *mut UnicodeString, a: &dyn Allocator) {
    // SAFETY: the caller's contract.
    let s = unsafe { &mut *s };
    if !s.buffer.is_null() {
        a.free(s.buffer as *mut u8);
    }
    *s = UnicodeString::EMPTY;
}

/// `RtlFreeAnsiString`.
///
/// # Safety
/// As [`free_unicode_string`].
pub unsafe fn free_ansi_string(s: *mut AnsiString, a: &dyn Allocator) {
    // SAFETY: the caller's contract.
    let s = unsafe { &mut *s };
    if !s.buffer.is_null() {
        a.free(s.buffer);
    }
    *s = AnsiString::EMPTY;
}

/// Append `units` to `dest`, or `STATUS_BUFFER_TOO_SMALL` (nothing
/// changes).
///
/// # Safety
/// The module's contract; `units` does not overlap `dest`'s free space.
unsafe fn append_units(dest: *mut UnicodeString, units: *const u16, count: usize) -> NtStatus {
    // SAFETY: the caller's contract.
    let d = unsafe { &mut *dest };
    let bytes = count * 2;
    let len = usize::from(d.length) & !1;
    if len + bytes > usize::from(d.maximum_length) {
        return STATUS_BUFFER_TOO_SMALL;
    }
    if bytes > 0 {
        // SAFETY: room checked above.
        unsafe { ptr::copy(units, d.buffer.add(len / 2), count) };
    }
    d.length = (len + bytes) as u16;
    // SAFETY: the caller's contract.
    unsafe { terminate_unicode(d) };
    STATUS_SUCCESS
}

/// `RtlAppendUnicodeToString`: append a NUL-terminated string (null:
/// nothing).
///
/// # Safety
/// The module's contract.
pub unsafe fn append_unicode_to_string(dest: *mut UnicodeString, source: *const u16) -> NtStatus {
    if source.is_null() {
        return STATUS_SUCCESS;
    }
    // SAFETY: the caller's contract.
    let n = unsafe { wcslen(source, MAX_BYTES / 2) };
    // SAFETY: the caller's contract.
    unsafe { append_units(dest, source, n) }
}

/// `RtlAppendUnicodeStringToString`.
///
/// # Safety
/// The module's contract.
pub unsafe fn append_unicode_string_to_string(dest: *mut UnicodeString, source: *const UnicodeString) -> NtStatus {
    // SAFETY: the caller's contract.
    let s = unsafe { *source };
    // SAFETY: the caller's contract.
    unsafe { append_units(dest, s.buffer, usize::from(s.length) / 2) }
}

/// `RtlCreateUnicodeString`: `dest` gets a new, NUL-terminated copy of
/// `source`. Returns whether it could allocate.
///
/// # Safety
/// The module's contract; `source` is NUL-terminated.
pub unsafe fn create_unicode_string(dest: *mut UnicodeString, source: *const u16, a: &dyn Allocator) -> bool {
    // SAFETY: the caller's contract.
    let n = unsafe { wcslen(source, MAX_BYTES / 2) }.min(MAX_BYTES / 2 - 1);
    let size = (n + 1) * 2;
    let p = a.alloc(size) as *mut u16;
    if p.is_null() {
        return false;
    }
    // SAFETY: `p` holds `n + 1` units; `source` holds `n`.
    unsafe {
        ptr::copy_nonoverlapping(source, p, n);
        p.add(n).write(0);
        dest.write(UnicodeString { length: (n * 2) as u16, maximum_length: size as u16, buffer: p });
    }
    true
}

/// `RtlMultiByteToUnicodeN`: convert up to `mb_bytes` bytes into at most
/// `max_out` bytes of UTF-16; returns the bytes written.
///
/// # Safety
/// `out` holds `max_out` bytes, `mb` holds `mb_bytes`.
pub unsafe fn multibyte_to_unicode_n(out: *mut u16, max_out: usize, mb: *const u8, mb_bytes: usize) -> usize {
    let n = mb_bytes.min(max_out / 2);
    for i in 0..n {
        // SAFETY: the caller's contract, `i` < both lengths.
        unsafe { out.add(i).write(codepage::to_unicode(mb.add(i).read())) };
    }
    n * 2
}

/// `RtlUnicodeToMultiByteN`: convert up to `unicode_bytes` bytes of UTF-16
/// into at most `max_out` bytes; returns the bytes written.
///
/// # Safety
/// `out` holds `max_out` bytes, `unicode` holds `unicode_bytes`.
pub unsafe fn unicode_to_multibyte_n(out: *mut u8, max_out: usize, unicode: *const u16, unicode_bytes: usize) -> usize {
    let n = (unicode_bytes / 2).min(max_out);
    for i in 0..n {
        // SAFETY: the caller's contract.
        unsafe { out.add(i).write(codepage::from_unicode(unicode.add(i).read())) };
    }
    n
}

/// `RtlCompareString` (ANSI; case folded by the code page's upcase).
///
/// # Safety
/// The module's contract.
pub unsafe fn compare_string(a: *const AnsiString, b: *const AnsiString, ignore_case: bool) -> i32 {
    // SAFETY: the caller's contract.
    let (a, b) = unsafe { ((*a).bytes(), (*b).bytes()) };
    for (&x, &y) in a.iter().zip(b) {
        let (x, y) = if ignore_case { (codepage::upcase(x), codepage::upcase(y)) } else { (x, y) };
        if x != y {
            return i32::from(x) - i32::from(y);
        }
    }
    (a.len() as i32) - (b.len() as i32)
}

/// `RtlEqualString`.
///
/// # Safety
/// The module's contract.
pub unsafe fn equal_string(a: *const AnsiString, b: *const AnsiString, ignore_case: bool) -> bool {
    // SAFETY: the caller's contract.
    unsafe { (*a).length == (*b).length && compare_string(a, b, ignore_case) == 0 }
}

/// `RtlIntegerToUnicodeString`: `value` in `base` (0 means 10; 2, 8, 10
/// and 16 are valid, else `STATUS_INVALID_PARAMETER`), upper-case digits,
/// NUL-terminated if it fits; `STATUS_BUFFER_OVERFLOW` if the digits do
/// not fit.
///
/// # Safety
/// The module's contract.
pub unsafe fn integer_to_unicode_string(value: u32, base: u32, dest: *mut UnicodeString) -> NtStatus {
    let base = if base == 0 { 10 } else { base };
    if !matches!(base, 2 | 8 | 10 | 16) {
        return STATUS_INVALID_PARAMETER;
    }
    let mut digits = [0u16; 32];
    let mut n = 0;
    let mut v = value;
    loop {
        let d = (v % base) as u8;
        digits[n] = u16::from(if d < 10 { b'0' + d } else { b'A' + d - 10 });
        n += 1;
        v /= base;
        if v == 0 {
            break;
        }
    }
    // SAFETY: the caller's contract.
    let d = unsafe { &mut *dest };
    if n * 2 > usize::from(d.maximum_length) {
        return STATUS_BUFFER_OVERFLOW;
    }
    for i in 0..n {
        // SAFETY: room checked above.
        unsafe { d.buffer.add(i).write(digits[n - 1 - i]) };
    }
    d.length = (n * 2) as u16;
    // SAFETY: the caller's contract.
    unsafe { terminate_unicode(d) };
    STATUS_SUCCESS
}

/// `RtlUnicodeStringToInteger`: leading white space, an optional sign,
/// then digits in `base` (0: a `0x`, `0o` or `0b` prefix chooses 16, 8 or
/// 2, else 10), up to the first character that is not a digit of the
/// base. Learn's documented bases are 0, 2, 8, 10 and 16.
///
/// # Safety
/// The module's contract; `value` is writable.
pub unsafe fn unicode_string_to_integer(s: *const UnicodeString, base: u32, value: *mut u32) -> NtStatus {
    if !matches!(base, 0 | 2 | 8 | 10 | 16) {
        return STATUS_INVALID_PARAMETER;
    }
    // SAFETY: the caller's contract.
    let units = unsafe { (*s).units() };
    let mut i = 0;
    while i < units.len() && (units[i] == u16::from(b' ') || (0x09..=0x0D).contains(&units[i])) {
        i += 1;
    }
    let mut negative = false;
    if i < units.len() && (units[i] == u16::from(b'-') || units[i] == u16::from(b'+')) {
        negative = units[i] == u16::from(b'-');
        i += 1;
    }
    let mut base = base;
    if base == 0 {
        base = 10;
        if i + 1 < units.len() && units[i] == u16::from(b'0') {
            let p = units[i + 1] | 0x20;
            let chosen = match p {
                0x78 => Some(16), // 'x'
                0x6F => Some(8),  // 'o'
                0x62 => Some(2),  // 'b'
                _ => None,
            };
            if let Some(b) = chosen {
                base = b;
                i += 2;
            }
        }
    }
    let mut v: u32 = 0;
    while i < units.len() {
        let c = units[i];
        let d = match c {
            0x30..=0x39 => u32::from(c - 0x30),
            0x41..=0x46 => u32::from(c - 0x41) + 10,
            0x61..=0x66 => u32::from(c - 0x61) + 10,
            _ => break,
        };
        if d >= base {
            break;
        }
        v = v.wrapping_mul(base).wrapping_add(d);
        i += 1;
    }
    // SAFETY: the caller's contract.
    unsafe { value.write(if negative { v.wrapping_neg() } else { v }) };
    STATUS_SUCCESS
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::cell::RefCell;
    use std::vec::Vec;

    /// Allocations from boxed vectors, tracked so the tests see leaks.
    #[derive(Default)]
    struct Boxes(RefCell<Vec<(usize, std::boxed::Box<[u8]>)>>);

    impl Allocator for Boxes {
        fn alloc(&self, bytes: usize) -> *mut u8 {
            let mut b = std::vec![0xAAu8; bytes].into_boxed_slice();
            let p = b.as_mut_ptr();
            self.0.borrow_mut().push((p as usize, b));
            p
        }
        fn free(&self, p: *mut u8) {
            let mut v = self.0.borrow_mut();
            let i = v.iter().position(|(a, _)| *a == p as usize).expect("freed what was allocated");
            v.remove(i);
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    fn text(s: &UnicodeString) -> std::string::String {
        // SAFETY: the tests' strings are valid.
        std::string::String::from_utf16(unsafe { s.units() }).unwrap()
    }

    #[test]
    fn init_counts_bytes() {
        let w = wide("Poly");
        let mut s = UnicodeString::EMPTY;
        // SAFETY: valid pointers.
        unsafe { init_unicode_string(&mut s, w.as_ptr()) };
        assert_eq!((s.length, s.maximum_length), (8, 10));
        // SAFETY: as above.
        unsafe { init_unicode_string(&mut s, ptr::null()) };
        assert_eq!((s.length, s.maximum_length, s.buffer.is_null()), (0, 0, true));
        let mut a = AnsiString::EMPTY;
        // SAFETY: as above.
        unsafe { init_ansi_string(&mut a, c"abc".as_ptr().cast()) };
        assert_eq!((a.length, a.maximum_length), (3, 4));
        let long: Vec<u16> = std::iter::repeat_n(b'x' as u16, 40000).chain([0]).collect();
        // SAFETY: as above.
        assert_eq!(unsafe { init_unicode_string_ex(&mut s, long.as_ptr()) }, STATUS_NAME_TOO_LONG);
        // SAFETY: as above.
        unsafe { init_unicode_string(&mut s, long.as_ptr()) };
        assert_eq!(usize::from(s.length), MAX_BYTES - 2);
    }

    #[test]
    fn copy_truncates_to_the_destination() {
        let w = wide("Hello, world");
        let mut src = UnicodeString::EMPTY;
        // SAFETY: valid pointers.
        unsafe { init_unicode_string(&mut src, w.as_ptr()) };
        let mut buf = [0xFFFFu16; 6];
        let mut d = UnicodeString { length: 0, maximum_length: 10, buffer: buf.as_mut_ptr() };
        // SAFETY: as above.
        unsafe { copy_unicode_string(&mut d, &src) };
        assert_eq!(text(&d), "Hello");
        assert_eq!(buf[5], 0xFFFF, "no NUL past MaximumLength");
        d.maximum_length = 12;
        // SAFETY: as above.
        unsafe { copy_unicode_string(&mut d, &src) };
        assert_eq!(text(&d), "Hello,");
        let mut room = [0xFFFFu16; 8];
        let short = wide("Hi");
        let mut s2 = UnicodeString::EMPTY;
        let mut d2 = UnicodeString { length: 0, maximum_length: 16, buffer: room.as_mut_ptr() };
        // SAFETY: as above.
        unsafe {
            init_unicode_string(&mut s2, short.as_ptr());
            copy_unicode_string(&mut d2, &s2);
        }
        assert_eq!(&room[..3], &[b'H' as u16, b'i' as u16, 0], "a NUL when it fits");
        // SAFETY: as above.
        unsafe { copy_unicode_string(&mut d, ptr::null()) };
        assert_eq!(d.length, 0);
    }

    #[test]
    fn compare_equal_and_prefix() {
        let (a, b, c) = (wide("POLY"), wide("poly"), wide("polyglot"));
        let mut s = [UnicodeString::EMPTY; 3];
        // SAFETY: valid pointers.
        unsafe {
            init_unicode_string(&mut s[0], a.as_ptr());
            init_unicode_string(&mut s[1], b.as_ptr());
            init_unicode_string(&mut s[2], c.as_ptr());
            assert_eq!(compare_unicode_string(&s[0], &s[1], true), 0);
            assert!(compare_unicode_string(&s[0], &s[1], false) < 0);
            assert!(compare_unicode_string(&s[1], &s[2], false) < 0);
            assert!(equal_unicode_string(&s[0], &s[1], true));
            assert!(!equal_unicode_string(&s[0], &s[1], false));
            assert!(prefix_unicode_string(&s[0], &s[2], true));
            assert!(!prefix_unicode_string(&s[2], &s[0], true));
        }
    }

    #[test]
    fn case_mapping() {
        assert_eq!(upcase_char(b'q' as u16), b'Q' as u16);
        assert_eq!(downcase_char(b'Q' as u16), b'q' as u16);
        assert_eq!(downcase_char(0x416), 0x436); // Ж → ж
        assert_eq!(downcase_char(0xD800), 0xD800);
        let w = wide("Mixed Case");
        let mut src = UnicodeString::EMPTY;
        let a = Boxes::default();
        let mut d = UnicodeString::EMPTY;
        // SAFETY: valid pointers.
        unsafe {
            init_unicode_string(&mut src, w.as_ptr());
            assert_eq!(case_unicode_string(&mut d, &src, true, true, &a), STATUS_SUCCESS);
            assert_eq!(text(&d), "MIXED CASE");
            free_unicode_string(&mut d, &a);
            let mut small = [0u16; 2];
            let mut d = UnicodeString { length: 0, maximum_length: 4, buffer: small.as_mut_ptr() };
            assert_eq!(case_unicode_string(&mut d, &src, false, false, &a), STATUS_BUFFER_OVERFLOW);
        }
        assert!(a.0.borrow().is_empty());
    }

    #[test]
    fn in_place_upcase() {
        let mut w = wide("abc");
        let mut s = UnicodeString { length: 6, maximum_length: 8, buffer: w.as_mut_ptr() };
        let a = Boxes::default();
        let p: *mut UnicodeString = &mut s;
        // SAFETY: the same string as source and destination is allowed.
        assert_eq!(unsafe { case_unicode_string(p, p, false, true, &a) }, STATUS_SUCCESS);
        assert_eq!(text(&s), "ABC");
    }

    #[test]
    fn ansi_and_unicode_conversions() {
        let a = Boxes::default();
        let bytes = [b'C', b'a', b'f', 0xE9, 0x80];
        let src = AnsiString { length: 5, maximum_length: 5, buffer: bytes.as_ptr() as *mut u8 };
        let mut u = UnicodeString::EMPTY;
        // SAFETY: valid pointers.
        unsafe {
            assert_eq!(ansi_to_unicode_size(&src), 12);
            assert_eq!(ansi_to_unicode_string(&mut u, &src, true, &a), STATUS_SUCCESS);
            assert_eq!(text(&u), "Café€");
            assert_eq!(u.maximum_length, 12);
            assert_eq!(*u.buffer.add(5), 0);
            let mut back = AnsiString::EMPTY;
            assert_eq!(unicode_to_ansi_string(&mut back, &u, true, &a), STATUS_SUCCESS);
            assert_eq!(back.bytes(), &bytes);
            free_ansi_string(&mut back, &a);
            let mut small = [0u8; 3];
            let mut back = AnsiString { length: 0, maximum_length: 3, buffer: small.as_mut_ptr() };
            assert_eq!(unicode_to_ansi_string(&mut back, &u, false, &a), STATUS_BUFFER_OVERFLOW);
            assert_eq!(back.bytes(), b"Ca");
            free_unicode_string(&mut u, &a);
            let mut small = [0u16; 2];
            let mut u = UnicodeString { length: 0, maximum_length: 4, buffer: small.as_mut_ptr() };
            assert_eq!(ansi_to_unicode_string(&mut u, &src, false, &a), STATUS_BUFFER_OVERFLOW);
        }
        assert!(a.0.borrow().is_empty());
    }

    #[test]
    fn n_conversions_stop_at_either_limit() {
        let mut out = [0u16; 4];
        let mb = b"abcdef";
        // SAFETY: valid buffers of the sizes given.
        unsafe {
            assert_eq!(multibyte_to_unicode_n(out.as_mut_ptr(), 8, mb.as_ptr(), 6), 8);
            assert_eq!(out, [b'a' as u16, b'b' as u16, b'c' as u16, b'd' as u16]);
            let mut back = [0u8; 8];
            let w = wide("xyz€");
            assert_eq!(unicode_to_multibyte_n(back.as_mut_ptr(), 8, w.as_ptr(), 8), 4);
            assert_eq!(&back[..4], b"xyz\x80");
        }
    }

    #[test]
    fn append_and_create() {
        let a = Boxes::default();
        let mut buf = [0u16; 8];
        let mut d = UnicodeString { length: 0, maximum_length: 16, buffer: buf.as_mut_ptr() };
        let (x, y) = (wide("abc"), wide("defgh"));
        // SAFETY: valid pointers.
        unsafe {
            assert_eq!(append_unicode_to_string(&mut d, x.as_ptr()), STATUS_SUCCESS);
            assert_eq!(text(&d), "abc");
            let mut ys = UnicodeString::EMPTY;
            init_unicode_string(&mut ys, y.as_ptr());
            assert_eq!(append_unicode_string_to_string(&mut d, &ys), STATUS_SUCCESS);
            assert_eq!(text(&d), "abcdefgh");
            assert_eq!(append_unicode_to_string(&mut d, x.as_ptr()), STATUS_BUFFER_TOO_SMALL);
            assert_eq!(text(&d), "abcdefgh");
            let mut c = UnicodeString::EMPTY;
            assert!(create_unicode_string(&mut c, y.as_ptr(), &a));
            assert_eq!((text(&c).as_str(), c.maximum_length), ("defgh", 12));
            free_unicode_string(&mut c, &a);
        }
        assert!(a.0.borrow().is_empty());
    }

    #[test]
    fn ansi_compare() {
        let (x, y) = (*b"hello", *b"HELLO");
        let a = AnsiString { length: 5, maximum_length: 5, buffer: x.as_ptr() as *mut u8 };
        let b = AnsiString { length: 5, maximum_length: 5, buffer: y.as_ptr() as *mut u8 };
        // SAFETY: valid strings.
        unsafe {
            assert!(equal_string(&a, &b, true));
            assert!(!equal_string(&a, &b, false));
            assert!(compare_string(&a, &b, false) > 0);
        }
    }

    #[test]
    fn integers() {
        let mut buf = [0u16; 12];
        let mut d = UnicodeString { length: 0, maximum_length: 24, buffer: buf.as_mut_ptr() };
        // SAFETY: valid pointers.
        unsafe {
            assert_eq!(integer_to_unicode_string(1234, 0, &mut d), STATUS_SUCCESS);
            assert_eq!(text(&d), "1234");
            assert_eq!(integer_to_unicode_string(0xBEEF, 16, &mut d), STATUS_SUCCESS);
            assert_eq!(text(&d), "BEEF");
            assert_eq!(integer_to_unicode_string(5, 2, &mut d), STATUS_SUCCESS);
            assert_eq!(text(&d), "101");
            assert_eq!(integer_to_unicode_string(5, 3, &mut d), STATUS_INVALID_PARAMETER);
            d.maximum_length = 4;
            assert_eq!(integer_to_unicode_string(12345, 10, &mut d), STATUS_BUFFER_OVERFLOW);
        }
        let parse = |s: &str, base: u32| -> (NtStatus, u32) {
            let w = wide(s);
            let mut us = UnicodeString::EMPTY;
            let mut v = 0;
            // SAFETY: valid pointers.
            let st = unsafe {
                init_unicode_string(&mut us, w.as_ptr());
                unicode_string_to_integer(&us, base, &mut v)
            };
            (st, v)
        };
        assert_eq!(parse("  42xyz", 0), (STATUS_SUCCESS, 42));
        assert_eq!(parse("0x1F", 0), (STATUS_SUCCESS, 31));
        assert_eq!(parse("0b101", 0), (STATUS_SUCCESS, 5));
        assert_eq!(parse("0o17", 0), (STATUS_SUCCESS, 15));
        assert_eq!(parse("-7", 10), (STATUS_SUCCESS, (-7i32) as u32));
        assert_eq!(parse("ff", 16), (STATUS_SUCCESS, 255));
        assert_eq!(parse("19", 8), (STATUS_SUCCESS, 1));
        assert_eq!(parse("", 10), (STATUS_SUCCESS, 0));
        assert_eq!(parse("1", 7).0, STATUS_INVALID_PARAMETER);
    }
}
