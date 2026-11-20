//! CFString and CFMutableString (`<CoreFoundation/CFString.h>`): UTF-16
//! code units, made from and converted to the encodings of
//! `poly_cf::encoding`; `CFSTR` constants as the compiler lays them out.

use crate::base::{self, Boolean, CFIndex, Header, Ref, id};
use alloc::vec::Vec;
use core::ffi::{c_char, c_void};
use poly_cf::{compare, encoding};

/// A `CFSTR` constant (spec sheet `__CFConstantStringClassReference`):
/// class word, flags, characters, length in units.
#[repr(C)]
pub struct ConstStr {
    pub isa: *const base::Class,
    pub flags: u32,
    pub pad: u32,
    pub ptr: *const u8,
    pub len: isize,
}

// SAFETY: constants are never written.
unsafe impl Sync for ConstStr {}

/// The flags of a constant whose characters are bytes; with bit 0x10 set
/// they are UTF-16 units.
pub const CONST_8BIT: u32 = 0x07C8;
const CONST_UTF16_BIT: u32 = 0x10;

/// A constant string of ASCII text, laid out as `CFSTR` does.
macro_rules! cfstr {
    ($text:expr) => {
        $crate::string::ConstStr {
            isa: &$crate::base::__CFConstantStringClassReference,
            flags: $crate::string::CONST_8BIT,
            pad: 0,
            ptr: concat!($text, "\0").as_ptr(),
            len: $text.len() as isize,
        }
    };
}
pub(crate) use cfstr;

/// A string made at run time.
#[repr(C)]
pub struct Str {
    h: Header,
    units: Vec<u16>,
}

/// `CFRange`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CFRange {
    pub location: CFIndex,
    pub length: CFIndex,
}

/// A string's units, borrowed or converted.
pub enum Units<'a> {
    Borrowed(&'a [u16]),
    Owned(Vec<u16>),
}

impl core::ops::Deref for Units<'_> {
    type Target = [u16];
    fn deref(&self) -> &[u16] {
        match self {
            Units::Borrowed(s) => s,
            Units::Owned(v) => v,
        }
    }
}

fn check_string(cf: Ref) {
    if cf.is_null() {
        base::crash("CFString: NULL string");
    }
}

/// The units of a string object.
pub fn units<'a>(cf: Ref) -> Units<'a> {
    check_string(cf);
    if base::is_constant_string(cf) {
        // SAFETY: a CFSTR constant: the compiler's layout.
        let c = unsafe { &*(cf as *const ConstStr) };
        let n = c.len.max(0) as usize;
        if c.flags & CONST_UTF16_BIT != 0 {
            let mut v = Vec::with_capacity(n);
            for i in 0..n {
                // SAFETY: `n` units at `ptr` (maybe unaligned in data).
                v.push(unsafe { (c.ptr as *const u16).add(i).read_unaligned() });
            }
            return Units::Owned(v);
        }
        // SAFETY: `n` bytes at `ptr`.
        let b = unsafe { core::slice::from_raw_parts(c.ptr, n) };
        return Units::Owned(b.iter().map(|&x| u16::from(x)).collect());
    }
    // SAFETY: a string object made here.
    let s = unsafe { &*(cf as *const Str) };
    Units::Borrowed(&s.units)
}

/// The string as Rust text (lone surrogates become U+FFFD).
pub fn text(cf: Ref) -> alloc::string::String {
    let mut out = Vec::new();
    encoding::to_utf8_lossy(&units(cf), &mut out);
    alloc::string::String::from_utf8(out).unwrap_or_default()
}

/// The units of a mutable string, for editing.
fn units_mut<'a>(cf: Ref) -> &'a mut Vec<u16> {
    check_string(cf);
    if !base::is_mutable(cf) {
        base::raise("NSInternalInconsistencyException", "mutating method sent to immutable object");
    }
    // SAFETY: a mutable string object made here; one thread.
    unsafe { &mut (*(cf as *mut Str)).units }
}

/// A new string object of `u`.
pub fn from_units(alloc: Ref, u: Vec<u16>, mutable: bool) -> Ref {
    base::make(alloc, id::STRING, mutable, |h| Str { h, units: u }) as Ref
}

pub fn from_str(s: &str) -> Ref {
    from_units(core::ptr::null(), s.encode_utf16().collect(), false)
}

/// # Safety
/// The count reached zero.
pub unsafe fn destroy(cf: Ref) {
    // SAFETY: a string object made by `from_units`.
    unsafe { base::destroy::<Str>(cf) }
}

/// The bytes of a NUL-terminated C string.
///
/// # Safety
/// `p` is a NUL-terminated string.
pub unsafe fn c_bytes<'a>(p: *const c_char) -> &'a [u8] {
    let mut n = 0;
    // SAFETY: the caller's contract.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p as *const u8, n)
    }
}

fn range_of(cf: Ref, r: CFRange) -> core::ops::Range<usize> {
    let len = units(cf).len();
    if r.location < 0 || r.length < 0 || (r.location + r.length) as usize > len {
        base::raise("NSRangeException", "range out of bounds");
    }
    r.location as usize..(r.location + r.length) as usize
}

// ----- creation -----

#[unsafe(no_mangle)]
pub extern "C" fn CFStringGetTypeID() -> usize {
    id::STRING
}

/// # Safety
/// `cstr` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringCreateWithCString(alloc: Ref, cstr: *const c_char, enc: u32) -> Ref {
    if cstr.is_null() {
        base::crash("CFStringCreateWithCString: NULL");
    }
    // SAFETY: the caller's contract.
    let b = unsafe { c_bytes(cstr) };
    match encoding::decode(b, enc, false) {
        Some(u) => from_units(alloc, u, false),
        None => core::ptr::null(),
    }
}

/// # Safety
/// As for `CFStringCreateWithCString`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringCreateWithCStringNoCopy(
    alloc: Ref,
    cstr: *const c_char,
    enc: u32,
    dealloc: Ref,
) -> Ref {
    // SAFETY: the caller's contract.
    let s = unsafe { CFStringCreateWithCString(alloc, cstr, enc) };
    free_buffer(dealloc, cstr as *mut c_void);
    s
}

/// The characters were copied: a `NoCopy` buffer goes back to its
/// deallocator at once.
fn free_buffer(dealloc: Ref, p: *mut c_void) {
    if dealloc != crate::allocator::null_allocator() {
        crate::allocator::deallocate(crate::allocator::resolve(dealloc), p);
    }
}

/// # Safety
/// `bytes` holds `n` bytes (or is anything when `n` is 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringCreateWithBytes(
    alloc: Ref,
    bytes: *const u8,
    n: CFIndex,
    enc: u32,
    external: Boolean,
) -> Ref {
    let b: &[u8] = if n <= 0 {
        &[]
    } else {
        if bytes.is_null() {
            base::crash("CFStringCreateWithBytes: NULL bytes");
        }
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(bytes, n as usize) }
    };
    match encoding::decode(b, enc, external != 0) {
        Some(u) => from_units(alloc, u, false),
        None => core::ptr::null(),
    }
}

/// # Safety
/// As for `CFStringCreateWithBytes`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringCreateWithBytesNoCopy(
    alloc: Ref,
    bytes: *const u8,
    n: CFIndex,
    enc: u32,
    external: Boolean,
    dealloc: Ref,
) -> Ref {
    // SAFETY: the caller's contract.
    let s = unsafe { CFStringCreateWithBytes(alloc, bytes, n, enc, external) };
    free_buffer(dealloc, bytes as *mut c_void);
    s
}

/// # Safety
/// `chars` holds `n` units.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringCreateWithCharacters(alloc: Ref, chars: *const u16, n: CFIndex) -> Ref {
    let u = if n <= 0 {
        Vec::new()
    } else {
        if chars.is_null() {
            base::crash("CFStringCreateWithCharacters: NULL characters");
        }
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(chars, n as usize) }.to_vec()
    };
    from_units(alloc, u, false)
}

/// # Safety
/// As for `CFStringCreateWithCharacters`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringCreateWithCharactersNoCopy(
    alloc: Ref,
    chars: *const u16,
    n: CFIndex,
    dealloc: Ref,
) -> Ref {
    // SAFETY: the caller's contract.
    let s = unsafe { CFStringCreateWithCharacters(alloc, chars, n) };
    free_buffer(dealloc, chars as *mut c_void);
    s
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringCreateCopy(alloc: Ref, s: Ref) -> Ref {
    if base::is_constant_string(s) || !base::is_mutable(s) {
        return base::retain(s);
    }
    from_units(alloc, units(s).to_vec(), false)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringCreateWithSubstring(alloc: Ref, s: Ref, r: CFRange) -> Ref {
    let range = range_of(s, r);
    from_units(alloc, units(s)[range].to_vec(), false)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringCreateMutable(alloc: Ref, _max: CFIndex) -> Ref {
    from_units(alloc, Vec::new(), true)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringCreateMutableCopy(alloc: Ref, _max: CFIndex, s: Ref) -> Ref {
    from_units(alloc, units(s).to_vec(), true)
}

// ----- reading -----

#[unsafe(no_mangle)]
pub extern "C" fn CFStringGetLength(s: Ref) -> CFIndex {
    if base::is_constant_string(s) {
        // SAFETY: a CFSTR constant.
        return unsafe { (*(s as *const ConstStr)).len };
    }
    units(s).len() as CFIndex
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringGetCharacterAtIndex(s: Ref, i: CFIndex) -> u16 {
    let u = units(s);
    match usize::try_from(i).ok().and_then(|i| u.get(i)) {
        Some(&c) => c,
        None => base::raise("NSRangeException", "index out of bounds"),
    }
}

/// # Safety
/// `buf` has room for `r.length` units.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringGetCharacters(s: Ref, r: CFRange, buf: *mut u16) {
    let range = range_of(s, r);
    let u = units(s);
    // SAFETY: the caller's contract.
    unsafe { core::ptr::copy_nonoverlapping(u[range].as_ptr(), buf, r.length as usize) };
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringGetCharactersPtr(s: Ref) -> *const u16 {
    if base::is_constant_string(s) || base::is_mutable(s) {
        return core::ptr::null();
    }
    match units(s) {
        Units::Borrowed(b) if !b.is_empty() => b.as_ptr(),
        _ => core::ptr::null(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringGetCStringPtr(s: Ref, enc: u32) -> *const c_char {
    if !base::is_constant_string(s) {
        return core::ptr::null();
    }
    // SAFETY: a CFSTR constant.
    let c = unsafe { &*(s as *const ConstStr) };
    if c.flags & CONST_UTF16_BIT != 0
        || !matches!(
            enc,
            encoding::ASCII
                | encoding::UTF8
                | encoding::MAC_ROMAN
                | encoding::ISO_LATIN1
                | encoding::WINDOWS_LATIN1
                | encoding::NEXTSTEP_LATIN
        )
    {
        return core::ptr::null();
    }
    // SAFETY: `len` bytes and a NUL at `ptr`.
    let b = unsafe { core::slice::from_raw_parts(c.ptr, c.len.max(0) as usize) };
    if b.iter().all(|&x| x < 0x80) { c.ptr as *const c_char } else { core::ptr::null() }
}

/// # Safety
/// `buf` has `size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringGetCString(s: Ref, buf: *mut c_char, size: CFIndex, enc: u32) -> Boolean {
    let mut out = Vec::new();
    let ok = encoding::encode(&units(s), enc, true, &mut out);
    if size <= 0 {
        return 0;
    }
    if !ok || out.len() + 1 > size as usize {
        // SAFETY: at least one byte.
        unsafe { *buf = 0 };
        return 0;
    }
    // SAFETY: `out.len() + 1 <= size` bytes.
    unsafe {
        core::ptr::copy_nonoverlapping(out.as_ptr(), buf as *mut u8, out.len());
        *buf.add(out.len()) = 0;
    }
    1
}

/// # Safety
/// `buf` (when not null) has `max` bytes; `used` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringGetBytes(
    s: Ref,
    r: CFRange,
    enc: u32,
    loss: u8,
    external: Boolean,
    buf: *mut u8,
    max: CFIndex,
    used: *mut CFIndex,
) -> CFIndex {
    let range = range_of(s, r);
    let u = units(s);
    let mut out: Vec<u8> = Vec::new();
    let mut done = 0usize;
    let limit = if buf.is_null() { usize::MAX } else { max.max(0) as usize };
    let mut i = range.start;
    if external != 0 && matches!(enc, encoding::UTF16 | encoding::UTF32) {
        let mut bom = Vec::new();
        encoding::encode(&[], enc, true, &mut bom);
        if bom.len() <= limit {
            out.extend_from_slice(&bom);
        }
    }
    while i < range.end {
        let n = if (0xD800..0xDC00).contains(&u[i]) && i + 1 < range.end { 2 } else { 1 };
        let mut piece = Vec::new();
        if !encoding::encode(&u[i..i + n], enc, false, &mut piece) {
            if loss == 0 {
                break;
            }
            piece.clear();
            piece.push(loss);
        }
        if out.len() + piece.len() > limit {
            break;
        }
        out.extend_from_slice(&piece);
        i += n;
        done += n;
    }
    if !buf.is_null() {
        // SAFETY: `out.len() <= max` bytes.
        unsafe { core::ptr::copy_nonoverlapping(out.as_ptr(), buf, out.len()) };
    }
    if !used.is_null() {
        // SAFETY: the caller's contract.
        unsafe { *used = out.len() as CFIndex };
    }
    done as CFIndex
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringCreateExternalRepresentation(alloc: Ref, s: Ref, enc: u32, loss: u8) -> Ref {
    let u = units(s);
    let mut out = Vec::new();
    if !encoding::encode(&u, enc, true, &mut out) {
        if loss == 0 {
            return core::ptr::null();
        }
        out.clear();
        for &c in u.iter() {
            if !encoding::encode(&[c], enc, false, &mut out) {
                out.push(loss);
            }
        }
    }
    crate::data::from_vec(alloc, out, false)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringGetMaximumSizeForEncoding(n: CFIndex, enc: u32) -> CFIndex {
    encoding::max_size(n as i64, enc) as CFIndex
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringGetSystemEncoding() -> u32 {
    encoding::MAC_ROMAN
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringGetFastestEncoding(_s: Ref) -> u32 {
    encoding::UTF16
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringGetSmallestEncoding(s: Ref) -> u32 {
    if units(s).iter().all(|&c| c < 0x80) { encoding::ASCII } else { encoding::UTF16 }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringIsEncodingAvailable(enc: u32) -> Boolean {
    encoding::is_available(enc) as Boolean
}

/// # Safety
/// `buf` has `max` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringGetFileSystemRepresentation(s: Ref, buf: *mut c_char, max: CFIndex) -> Boolean {
    // SAFETY: the caller's contract.
    unsafe { CFStringGetCString(s, buf, max, encoding::UTF8) }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringGetMaximumSizeOfFileSystemRepresentation(s: Ref) -> CFIndex {
    CFStringGetLength(s) * 3 + 1
}

/// # Safety
/// `p` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringCreateWithFileSystemRepresentation(alloc: Ref, p: *const c_char) -> Ref {
    // SAFETY: the caller's contract.
    unsafe { CFStringCreateWithCString(alloc, p, encoding::UTF8) }
}

// ----- comparing and searching -----

#[unsafe(no_mangle)]
pub extern "C" fn CFStringCompareWithOptions(a: Ref, b: Ref, r: CFRange, options: u64) -> CFIndex {
    let range = range_of(a, r);
    compare::compare(&units(a)[range], &units(b), options) as CFIndex
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringCompareWithOptionsAndLocale(
    a: Ref,
    b: Ref,
    r: CFRange,
    options: u64,
    _locale: Ref,
) -> CFIndex {
    CFStringCompareWithOptions(a, b, r, options)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringCompare(a: Ref, b: Ref, options: u64) -> CFIndex {
    compare::compare(&units(a), &units(b), options) as CFIndex
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringHasPrefix(s: Ref, p: Ref) -> Boolean {
    let (s, p) = (units(s), units(p));
    (!p.is_empty() && s.starts_with(&p)) as Boolean
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringHasSuffix(s: Ref, p: Ref) -> Boolean {
    let (s, p) = (units(s), units(p));
    (!p.is_empty() && s.ends_with(&p)) as Boolean
}

/// # Safety
/// `result` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringFindWithOptions(
    s: Ref,
    find: Ref,
    r: CFRange,
    options: u64,
    result: *mut CFRange,
) -> Boolean {
    let range = range_of(s, r);
    let (hay, needle) = (units(s), units(find));
    match compare::find(&hay[range.clone()], &needle, options) {
        Some(i) => {
            if !result.is_null() {
                // SAFETY: the caller's contract.
                unsafe {
                    *result = CFRange { location: (range.start + i) as CFIndex, length: needle.len() as CFIndex }
                };
            }
            1
        }
        None => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringFind(s: Ref, find: Ref, options: u64) -> CFRange {
    let mut r = CFRange { location: -1, length: 0 };
    let all = CFRange { location: 0, length: CFStringGetLength(s) };
    // SAFETY: a local out value.
    unsafe { CFStringFindWithOptions(s, find, all, options, &mut r) };
    r
}

// ----- editing -----

#[unsafe(no_mangle)]
pub extern "C" fn CFStringAppend(s: Ref, other: Ref) {
    let add = units(other).to_vec();
    units_mut(s).extend_from_slice(&add);
}

/// # Safety
/// `cstr` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringAppendCString(s: Ref, cstr: *const c_char, enc: u32) {
    // SAFETY: the caller's contract.
    let b = unsafe { c_bytes(cstr) };
    if let Some(u) = encoding::decode(b, enc, false) {
        units_mut(s).extend_from_slice(&u);
    }
}

/// # Safety
/// `chars` holds `n` units.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringAppendCharacters(s: Ref, chars: *const u16, n: CFIndex) {
    if n > 0 {
        // SAFETY: the caller's contract.
        let add = unsafe { core::slice::from_raw_parts(chars, n as usize) };
        units_mut(s).extend_from_slice(add);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringInsert(s: Ref, at: CFIndex, other: Ref) {
    let add = units(other).to_vec();
    let u = units_mut(s);
    if at < 0 || at as usize > u.len() {
        base::raise("NSRangeException", "index out of bounds");
    }
    u.splice(at as usize..at as usize, add);
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringDelete(s: Ref, r: CFRange) {
    let range = range_of(s, r);
    units_mut(s).drain(range);
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringReplace(s: Ref, r: CFRange, with: Ref) {
    let range = range_of(s, r);
    let add = units(with).to_vec();
    units_mut(s).splice(range, add);
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringReplaceAll(s: Ref, with: Ref) {
    let add = units(with).to_vec();
    *units_mut(s) = add;
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringFindAndReplace(s: Ref, find: Ref, with: Ref, r: CFRange, options: u64) -> CFIndex {
    let range = range_of(s, r);
    let (needle, rep) = (units(find).to_vec(), units(with).to_vec());
    if needle.is_empty() {
        return 0;
    }
    let u = units_mut(s);
    let mut out = Vec::with_capacity(u.len());
    out.extend_from_slice(&u[..range.start]);
    let (mut i, mut count) = (range.start, 0);
    while i < range.end {
        if i + needle.len() <= range.end && compare::equal_at(&u[i..i + needle.len()], &needle, options) {
            out.extend_from_slice(&rep);
            i += needle.len();
            count += 1;
        } else {
            out.push(u[i]);
            i += 1;
        }
    }
    out.extend_from_slice(&u[range.end..]);
    *u = out;
    count
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringLowercase(s: Ref, _locale: Ref) {
    for c in units_mut(s).iter_mut() {
        *c = compare::to_lower(*c);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringUppercase(s: Ref, _locale: Ref) {
    for c in units_mut(s).iter_mut() {
        *c = compare::to_upper(*c);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringTrimWhitespace(s: Ref) {
    let u = units_mut(s);
    let ws = |c: &u16| matches!(*c, 0x20 | 0x09 | 0x0A | 0x0D | 0xA0);
    let end = u.iter().rposition(|c| !ws(c)).map_or(0, |i| i + 1);
    u.truncate(end);
    let start = u.iter().position(|c| !ws(c)).unwrap_or(u.len());
    u.drain(..start);
}

// ----- numbers in text -----

fn leading_number(s: Ref) -> alloc::string::String {
    let t = text(s);
    let t = t.trim_start();
    let mut end = 0;
    for (i, c) in t.char_indices() {
        if c.is_ascii_digit() || (i == 0 && (c == '-' || c == '+')) || c == '.' || c == 'e' || c == 'E' {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    alloc::string::String::from(&t[..end])
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringGetIntValue(s: Ref) -> i32 {
    let t = leading_number(s);
    let digits: alloc::string::String = t
        .chars()
        .enumerate()
        .take_while(|(i, c)| c.is_ascii_digit() || (*i == 0 && (*c == '-' || *c == '+')))
        .map(|(_, c)| c)
        .collect();
    digits.parse::<i64>().map_or(0, |v| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringGetDoubleValue(s: Ref) -> f64 {
    let mut t = leading_number(s);
    while !t.is_empty() {
        if let Ok(v) = t.parse::<f64>() {
            return v;
        }
        t.pop();
    }
    0.0
}

// ----- arrays of strings -----

#[unsafe(no_mangle)]
pub extern "C" fn CFStringCreateArrayBySeparatingStrings(alloc: Ref, s: Ref, sep: Ref) -> Ref {
    let (u, d) = (units(s), units(sep));
    let mut parts: Vec<Ref> = Vec::new();
    if d.is_empty() {
        parts.push(from_units(alloc, u.to_vec(), false));
    } else {
        let mut start = 0;
        let mut i = 0;
        while i + d.len() <= u.len() {
            if u[i..i + d.len()] == d[..] {
                parts.push(from_units(alloc, u[start..i].to_vec(), false));
                i += d.len();
                start = i;
            } else {
                i += 1;
            }
        }
        parts.push(from_units(alloc, u[start..].to_vec(), false));
    }
    let a = crate::array::from_refs(alloc, &parts, &crate::array::TYPE_CALLBACKS, false);
    for p in parts {
        base::release(p);
    }
    a
}

#[unsafe(no_mangle)]
pub extern "C" fn CFStringCreateByCombiningStrings(alloc: Ref, array: Ref, sep: Ref) -> Ref {
    let d = units(sep).to_vec();
    let mut out = Vec::new();
    for (i, &e) in crate::array::items(array).iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(&d);
        }
        out.extend_from_slice(&units(e));
    }
    from_units(alloc, out, false)
}

// ----- exported constant strings -----

/// A constant string made here, as a `Ref`.
pub fn constant(s: &'static ConstStr) -> Ref {
    s as *const ConstStr as Ref
}
