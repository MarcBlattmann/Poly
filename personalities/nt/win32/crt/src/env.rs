//! The runtime's environment (Learn: `getenv`, `_putenv`, `getenv_s`,
//! `_dupenv_s`): the `_environ` / `_wenviron` arrays of `NAME=value`
//! strings. Names match without regard to ASCII case (Learn: "getenv ...
//! is not case sensitive" on Windows). `_putenv` changes the runtime's
//! arrays and the process environment (`SetEnvironmentVariableW`), so a
//! child process and the other width see it; `NAME=` removes `NAME`.
//! A changed array is a new block: `__initenv` keeps the start-up one, and
//! an old block stays allocated (pointers `getenv` gave stay valid).

use crate::global;
use crate::heap;
use crate::startup;
use crate::sys;
use alloc::vec::Vec;
use poly_crt::errno::{EINVAL, ERANGE};
use poly_crt::unit::{self, Unit};

fn eq_ci<C: Unit>(a: &[C], b: &[C]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| poly_crt::string::fold(*x) == poly_crt::string::fold(*y))
}

/// The entries of a NULL-terminated array.
///
/// # Safety
/// `env` is NULL or such an array of C strings.
unsafe fn entries<'a, C: Unit>(env: *mut *mut C) -> Vec<&'a [C]> {
    let mut v = Vec::new();
    if env.is_null() {
        return v;
    }
    let mut i = 0;
    // SAFETY: the array ends with NULL; each entry is a C string.
    unsafe {
        while !(*env.add(i)).is_null() {
            v.push(unit::slice(*env.add(i)));
            i += 1;
        }
    }
    v
}

/// The value of `name` in `env`.
///
/// # Safety
/// As [`entries`].
unsafe fn lookup<C: Unit>(env: *mut *mut C, name: &[C]) -> Option<*mut C> {
    // SAFETY: the caller's array.
    for e in unsafe { entries(env) } {
        if e.len() > name.len() && e[name.len()].to_u32() == u32::from(b'=') && eq_ci(&e[..name.len()], name) {
            return Some(e[name.len() + 1..].as_ptr() as *mut C);
        }
    }
    None
}

/// Replace, add or (`value` None) remove `name` in `*cell`.
///
/// # Safety
/// `cell` holds NULL or an environment array.
unsafe fn update<C: Unit>(cell: *mut *mut *mut C, name: &[C], value: Option<&[C]>) {
    // SAFETY: the caller's cell.
    let old = unsafe { *cell };
    if old.is_null() {
        return;
    }
    // SAFETY: as above.
    let mut list: Vec<*mut C> = unsafe { entries(old) }.iter().map(|e| e.as_ptr() as *mut C).collect();
    // SAFETY: entries are C strings.
    let pos = list.iter().position(|&p| {
        let e = unsafe { unit::slice(p) };
        e.len() > name.len() && e[name.len()].to_u32() == u32::from(b'=') && eq_ci(&e[..name.len()], name)
    });
    let new_entry = value.map(|v| {
        let mut s: Vec<C> = name.to_vec();
        s.push(C::ascii(b'='));
        s.extend_from_slice(v);
        heap::dup(&s)
    });
    match (pos, new_entry) {
        (Some(i), Some(p)) => list[i] = p,
        (Some(i), None) => {
            list.remove(i);
        }
        (None, Some(p)) => list.push(p),
        (None, None) => return,
    }
    let arr = heap::malloc((list.len() + 1) * 8) as *mut *mut C;
    if arr.is_null() {
        return;
    }
    // SAFETY: the new array holds list.len() + 1 pointers.
    unsafe {
        for (i, &p) in list.iter().enumerate() {
            arr.add(i).write(p);
        }
        arr.add(list.len()).write(core::ptr::null_mut());
        *cell = arr;
    }
}

/// Set (or remove) a variable everywhere.
fn put(name_w: &[u16], value_w: Option<&[u16]>) -> i32 {
    if name_w.is_empty() || name_w.contains(&u16::from(b'=')) {
        global::invalid(EINVAL);
        return -1;
    }
    let c = startup::cells();
    startup::_initialize_narrow_environment();
    let name_a = crate::text::wide_to_ansi(name_w);
    let value_a = value_w.map(crate::text::wide_to_ansi);
    // SAFETY: valid cells holding environment arrays.
    unsafe {
        update(c.environ, &name_a, value_a.as_deref());
        update(c.wenviron, name_w, value_w);
    }
    let mut n: Vec<u16> = name_w.to_vec();
    n.push(0);
    let v: Option<Vec<u16>> = value_w.map(|v| {
        let mut v = v.to_vec();
        v.push(0);
        v
    });
    // SAFETY: NUL-terminated strings.
    unsafe { sys::SetEnvironmentVariableW(n.as_ptr(), v.as_ref().map_or(core::ptr::null(), |v| v.as_ptr())) };
    0
}

/// `"NAME=value"` → `put`.
fn putenv_text(s: &[u16]) -> i32 {
    let Some(eq) = s.iter().position(|&c| c == u16::from(b'=')) else {
        global::invalid(EINVAL);
        return -1;
    };
    let value = &s[eq + 1..];
    put(&s[..eq], if value.is_empty() { None } else { Some(value) })
}

/// # Safety
/// `name` is a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getenv(name: *const u8) -> *mut u8 {
    if name.is_null() {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    startup::_initialize_narrow_environment();
    // SAFETY: a valid cell and the caller's string.
    unsafe { lookup(*startup::cells().environ, unit::slice(name)) }.unwrap_or(core::ptr::null_mut())
}

/// # Safety
/// As `getenv`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wgetenv(name: *const u16) -> *mut u16 {
    if name.is_null() {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    startup::_initialize_wide_environment();
    // SAFETY: a valid cell and the caller's string.
    unsafe { lookup(*startup::cells().wenviron, unit::slice(name)) }.unwrap_or(core::ptr::null_mut())
}

/// `getenv_s(&len, buf, size, name)`: `len` is the value's size with its
/// NUL (0 when absent).
///
/// # Safety
/// `len` writable; `buf` NULL or `size` units; `name` a C string.
unsafe fn getenv_s_impl<C: Unit>(len: *mut usize, buf: *mut C, size: usize, value: Option<*mut C>) -> i32 {
    // SAFETY: a found value is a C string.
    let v: &[C] = value.map_or(&[], |p| unsafe { unit::slice(p) });
    let need = if value.is_some() { v.len() + 1 } else { 0 };
    // SAFETY: writable.
    unsafe { *len = need };
    if buf.is_null() {
        return 0;
    }
    if need > size {
        global::invalid(ERANGE);
        return ERANGE;
    }
    // SAFETY: the buffer holds `size` >= need units (or at least 1 when 0).
    unsafe {
        core::ptr::copy_nonoverlapping(v.as_ptr(), buf, v.len());
        if size > 0 {
            buf.add(v.len()).write(C::default());
        }
    }
    0
}

/// # Safety
/// As `getenv_s_impl`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getenv_s(len: *mut usize, buf: *mut u8, size: usize, name: *const u8) -> i32 {
    if len.is_null() || name.is_null() || (buf.is_null() && size != 0) {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: the caller's arguments.
    unsafe { getenv_s_impl(len, buf, size, (!getenv(name).is_null()).then(|| getenv(name))) }
}

/// # Safety
/// As `getenv_s`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wgetenv_s(len: *mut usize, buf: *mut u16, size: usize, name: *const u16) -> i32 {
    if len.is_null() || name.is_null() || (buf.is_null() && size != 0) {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: the caller's arguments.
    unsafe { getenv_s_impl(len, buf, size, (!_wgetenv(name).is_null()).then(|| _wgetenv(name))) }
}

/// `_dupenv_s(&buf, &len, name)`: a heap copy, or NULL and 0 when absent.
///
/// # Safety
/// `out` writable; `len` NULL or writable; `name` a C string.
pub unsafe extern "C" fn _dupenv_s(out: *mut *mut u8, len: *mut usize, name: *const u8) -> i32 {
    if out.is_null() || name.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: the caller's arguments.
    unsafe {
        let v = getenv(name);
        let (p, n) =
            if v.is_null() { (core::ptr::null_mut(), 0) } else { (heap::dup(unit::slice(v)), unit::len(v) + 1) };
        *out = p;
        if !len.is_null() {
            *len = n;
        }
    }
    0
}

/// # Safety
/// As `_dupenv_s`.
pub unsafe extern "C" fn _wdupenv_s(out: *mut *mut u16, len: *mut usize, name: *const u16) -> i32 {
    if out.is_null() || name.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: the caller's arguments.
    unsafe {
        let v = _wgetenv(name);
        let (p, n) =
            if v.is_null() { (core::ptr::null_mut(), 0) } else { (heap::dup(unit::slice(v)), unit::len(v) + 1) };
        *out = p;
        if !len.is_null() {
            *len = n;
        }
    }
    0
}

/// # Safety
/// `s` is a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _putenv(s: *const u8) -> i32 {
    if s.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: the caller's string.
    putenv_text(&crate::text::ansi_to_wide(unsafe { unit::slice(s) }))
}

/// # Safety
/// As `_putenv`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wputenv(s: *const u16) -> i32 {
    if s.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: the caller's string.
    putenv_text(unsafe { unit::slice(s) })
}

/// `_putenv_s(name, value)`: an empty value removes; 0 or the `errno`.
///
/// # Safety
/// Valid C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _putenv_s(name: *const u8, value: *const u8) -> i32 {
    if name.is_null() || value.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: the caller's strings.
    let (n, v) =
        unsafe { (crate::text::ansi_to_wide(unit::slice(name)), crate::text::ansi_to_wide(unit::slice(value))) };
    if put(&n, if v.is_empty() { None } else { Some(&v) }) == 0 { 0 } else { EINVAL }
}

/// # Safety
/// As `_putenv_s`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wputenv_s(name: *const u16, value: *const u16) -> i32 {
    if name.is_null() || value.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: the caller's strings.
    let (n, v) = unsafe { (unit::slice(name), unit::slice(value)) };
    if put(n, if v.is_empty() { None } else { Some(v) }) == 0 { 0 } else { EINVAL }
}
