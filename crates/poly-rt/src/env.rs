//! The arguments and environment the program was started with, read off
//! its initial stack by the entry point: `argc`, then `argc` pointers to
//! NUL-terminated arguments and a null pointer, then the environment's
//! pointers (`NAME=value`) and a null pointer, as the kernel's loader lays
//! them out for every ELF program (ADR 0125 keeps that layout).

use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

static ARGC: AtomicUsize = AtomicUsize::new(0);
static ARGV: AtomicPtr<*const u8> = AtomicPtr::new(core::ptr::null_mut());
static ENVP: AtomicPtr<*const u8> = AtomicPtr::new(core::ptr::null_mut());

/// Remember the arguments and environment (the entry point calls this).
///
/// # Safety
///
/// `argv` must point to `argc` pointers to NUL-terminated strings, and
/// `envp` to such pointers ending with a null one, all of which stay valid
/// and unchanged for the rest of the program.
pub unsafe fn init(argc: usize, argv: *const *const u8, envp: *const *const u8) {
    ARGC.store(argc, Ordering::Relaxed);
    ARGV.store(argv.cast_mut(), Ordering::Relaxed);
    ENVP.store(envp.cast_mut(), Ordering::Relaxed);
}

/// The NUL-terminated string at `p`, without its NUL.
///
/// # Safety
///
/// `p` points to a NUL-terminated string valid for the rest of the program.
unsafe fn c_str(p: *const u8) -> &'static [u8] {
    let mut len = 0;
    // SAFETY: the string is NUL-terminated, so every byte up to the NUL
    // is readable.
    while unsafe { *p.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: `len` readable bytes at `p`, valid from now on.
    unsafe { core::slice::from_raw_parts(p, len) }
}

/// The arguments, `args()[0]` being the program's name by convention.
pub fn args() -> impl Iterator<Item = &'static [u8]> {
    let (argc, argv) = (ARGC.load(Ordering::Relaxed), ARGV.load(Ordering::Relaxed));
    // SAFETY: `init` was given `argc` valid pointers at `argv` (or none:
    // `argc` is 0 then).
    (0..argc).map(move |i| unsafe { c_str(*argv.add(i)) })
}

/// The arguments that are valid UTF-8, as strings.
pub fn args_str() -> impl Iterator<Item = &'static str> {
    args().filter_map(|a| core::str::from_utf8(a).ok())
}

/// The environment's `NAME=value` strings.
pub fn vars() -> impl Iterator<Item = &'static [u8]> {
    let mut at = ENVP.load(Ordering::Relaxed);
    core::iter::from_fn(move || {
        if at.is_null() {
            return None;
        }
        // SAFETY: `init` was given a null-terminated array at `envp`; `at`
        // never passes its null pointer.
        let p = unsafe { *at };
        if p.is_null() {
            return None;
        }
        // SAFETY: as above, the next entry exists (the null one at least).
        at = unsafe { at.add(1) };
        // SAFETY: a NUL-terminated string valid for the program's life.
        Some(unsafe { c_str(p) })
    })
}

/// The value of the environment variable `name`.
pub fn var(name: &str) -> Option<&'static str> {
    vars().find_map(|v| {
        let rest = v.strip_prefix(name.as_bytes())?.strip_prefix(b"=")?;
        core::str::from_utf8(rest).ok()
    })
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    #[test]
    fn reads_arguments_and_environment_like_the_entry_point() {
        // What the kernel's loader puts on the stack, leaked to live for
        // the "program".
        let strings: &'static [&'static [u8]] = &[b"/bin/hello\0", b"-v\0", b"HOME=/\0", b"POLY=1\0"];
        let p: std::vec::Vec<*const u8> = strings.iter().map(|s| s.as_ptr()).collect();
        let argv: &'static [*const u8] = std::boxed::Box::leak(std::vec![p[0], p[1], core::ptr::null()].into());
        let envp: &'static [*const u8] =
            std::boxed::Box::leak(std::vec![p[2], p[3], core::ptr::null()].into_boxed_slice());
        // SAFETY: the arrays and strings above are leaked, so valid forever.
        unsafe { init(2, argv.as_ptr(), envp.as_ptr()) };
        let a: std::vec::Vec<&[u8]> = args().collect();
        assert_eq!(a, [&b"/bin/hello"[..], b"-v"]);
        assert_eq!(args_str().nth(1), Some("-v"));
        assert_eq!(vars().count(), 2);
        assert_eq!(var("HOME"), Some("/"));
        assert_eq!(var("POLY"), Some("1"));
        assert_eq!(var("POL"), None);
        assert_eq!(var("PATH"), None);
    }
}
