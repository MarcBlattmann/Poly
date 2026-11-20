//! CFURL (`<CoreFoundation/CFURL.h>`): a URL string, optionally relative
//! to a base URL. The text work is `poly_cf::url`'s.

use crate::base::{self, Boolean, CFIndex, Header, Ref, id};
use crate::string;
use alloc::string::String;
use poly_cf::url as text;

#[repr(C)]
pub struct Url {
    h: Header,
    /// The URL string (a CFString, retained).
    string: Ref,
    /// The base URL (retained) or null.
    base: Ref,
}

fn get<'a>(cf: Ref) -> &'a Url {
    if base::type_of(cf) != id::URL {
        base::crash("not a CFURL");
    }
    // SAFETY: a URL object.
    unsafe { &*(cf as *const Url) }
}

pub fn string(cf: Ref) -> Ref {
    get(cf).string
}

fn text_of(cf: Ref) -> String {
    string::text(get(cf).string)
}

/// The absolute URL text (the string resolved against the base).
pub fn absolute_text(cf: Ref) -> String {
    let u = get(cf);
    let own = string::text(u.string);
    if u.base.is_null() { own } else { text::resolve(&absolute_text(u.base), &own) }
}

/// The absolute file system path of a file URL.
pub fn path(cf: Ref) -> Option<String> {
    text::file_system_path(&absolute_text(cf))
}

pub fn make(alloc: Ref, s: &str, base_url: Ref) -> Ref {
    let st = string::from_units(alloc, s.encode_utf16().collect(), false);
    base::make(alloc, id::URL, false, |h| Url { h, string: st, base: base::retain_opt(base_url) }) as Ref
}

/// A file URL of an absolute path.
pub fn from_path(alloc: Ref, path: &str, dir: bool) -> Ref {
    make(alloc, &text::from_path(path, dir), core::ptr::null())
}

/// # Safety
/// The count reached zero.
pub unsafe fn destroy(cf: Ref) {
    let u = get(cf);
    base::release(u.string);
    base::release_opt(u.base);
    // SAFETY: a URL made here.
    unsafe { base::destroy::<Url>(cf) }
}

pub fn equal(a: Ref, b: Ref) -> bool {
    let (x, y) = (get(a), get(b));
    base::equal(x.string, y.string)
        && match (x.base.is_null(), y.base.is_null()) {
            (true, true) => true,
            (false, false) => base::equal(x.base, y.base),
            _ => false,
        }
}

pub fn hash(cf: Ref) -> u64 {
    base::hash(get(cf).string)
}

// ----- exports -----

#[unsafe(no_mangle)]
pub extern "C" fn CFURLGetTypeID() -> usize {
    id::URL
}

#[unsafe(no_mangle)]
pub extern "C" fn CFURLCreateWithString(alloc: Ref, s: Ref, base_url: Ref) -> Ref {
    if s.is_null() {
        return core::ptr::null();
    }
    make(alloc, &string::text(s), base_url)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFURLCreateWithFileSystemPath(alloc: Ref, p: Ref, _style: CFIndex, dir: Boolean) -> Ref {
    if p.is_null() {
        return core::ptr::null();
    }
    let path = string::text(p);
    let abs = poly_cf::bundle::normalize(&crate::sys::cwd(), &path);
    from_path(alloc, &abs, dir != 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFURLCreateWithFileSystemPathRelativeToBase(
    alloc: Ref,
    p: Ref,
    style: CFIndex,
    dir: Boolean,
    base_url: Ref,
) -> Ref {
    let path = string::text(p);
    if path.starts_with('/') || base_url.is_null() {
        return CFURLCreateWithFileSystemPath(alloc, p, style, dir);
    }
    make(alloc, &text::from_path(&path, dir != 0), base_url)
}

/// # Safety
/// `buf` holds `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFURLCreateFromFileSystemRepresentation(
    alloc: Ref,
    buf: *const u8,
    n: CFIndex,
    dir: Boolean,
) -> Ref {
    // SAFETY: the caller's contract.
    let b = unsafe { core::slice::from_raw_parts(buf, n.max(0) as usize) };
    let path = String::from_utf8_lossy(b);
    let abs = poly_cf::bundle::normalize(&crate::sys::cwd(), &path);
    from_path(alloc, &abs, dir != 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFURLGetString(u: Ref) -> Ref {
    get(u).string
}

#[unsafe(no_mangle)]
pub extern "C" fn CFURLGetBaseURL(u: Ref) -> Ref {
    get(u).base
}

#[unsafe(no_mangle)]
pub extern "C" fn CFURLCopyAbsoluteURL(u: Ref) -> Ref {
    if get(u).base.is_null() {
        return base::retain(u);
    }
    make(core::ptr::null(), &absolute_text(u), core::ptr::null())
}

#[unsafe(no_mangle)]
pub extern "C" fn CFURLCopyFileSystemPath(u: Ref, _style: CFIndex) -> Ref {
    match text::file_system_path(&text_of(u)) {
        Some(p) => string::from_str(&p),
        None => core::ptr::null(),
    }
}

/// # Safety
/// `buf` has `max` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFURLGetFileSystemRepresentation(
    u: Ref,
    resolve: Boolean,
    buf: *mut u8,
    max: CFIndex,
) -> Boolean {
    let p = if resolve != 0 { path(u) } else { text::file_system_path(&text_of(u)) };
    let Some(p) = p else { return 0 };
    if p.len() + 1 > max.max(0) as usize {
        return 0;
    }
    // SAFETY: the caller's contract: room for the path and its NUL.
    unsafe {
        core::ptr::copy_nonoverlapping(p.as_ptr(), buf, p.len());
        *buf.add(p.len()) = 0;
    }
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn CFURLHasDirectoryPath(u: Ref) -> Boolean {
    text::path_part(&text_of(u)).ends_with('/') as Boolean
}

#[unsafe(no_mangle)]
pub extern "C" fn CFURLCopyScheme(u: Ref) -> Ref {
    match text::scheme(&absolute_text(u)) {
        Some(s) => string::from_str(s),
        None => core::ptr::null(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFURLCopyPath(u: Ref) -> Ref {
    string::from_str(text::path_part(&text_of(u)))
}

#[unsafe(no_mangle)]
pub extern "C" fn CFURLCopyLastPathComponent(u: Ref) -> Ref {
    let p = text::file_system_path(&absolute_text(u)).unwrap_or_default();
    string::from_str(poly_cf::bundle::last_component(&p))
}

#[unsafe(no_mangle)]
pub extern "C" fn CFURLCopyPathExtension(u: Ref) -> Ref {
    let p = text::file_system_path(&absolute_text(u)).unwrap_or_default();
    let last = poly_cf::bundle::last_component(&p);
    match last.rfind('.') {
        Some(i) if i > 0 => string::from_str(&last[i + 1..]),
        _ => core::ptr::null(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFURLCreateCopyDeletingLastPathComponent(alloc: Ref, u: Ref) -> Ref {
    make(alloc, &text::deleting_last_component(&absolute_text(u)), core::ptr::null())
}

#[unsafe(no_mangle)]
pub extern "C" fn CFURLCreateCopyAppendingPathComponent(alloc: Ref, u: Ref, c: Ref, dir: Boolean) -> Ref {
    make(alloc, &text::appending_component(&absolute_text(u), &string::text(c), dir != 0), core::ptr::null())
}

#[unsafe(no_mangle)]
pub extern "C" fn CFURLCreateCopyDeletingPathExtension(alloc: Ref, u: Ref) -> Ref {
    let s = absolute_text(u);
    let dir = s.ends_with('/');
    let body = s.trim_end_matches('/');
    let cut = match (body.rfind('.'), body.rfind('/')) {
        (Some(d), Some(sl)) if d > sl + 1 => &body[..d],
        _ => body,
    };
    let mut out = String::from(cut);
    if dir {
        out.push('/');
    }
    make(alloc, &out, core::ptr::null())
}
