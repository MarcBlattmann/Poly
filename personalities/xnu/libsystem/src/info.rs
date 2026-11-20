//! libsystem_info (ADR 0257 addendum): the user database. Poly has one
//! user so far: `getpwuid` and `getpwnam` answer for the caller's uid
//! with a home of `/` and the shell `/bin/sh` (`struct passwd`,
//! `<pwd.h>`).

use core::ffi::c_char;

unsafe extern "C" {
    fn getuid() -> u32;
    fn getgid() -> u32;
}

/// `struct passwd` (`<pwd.h>`).
#[repr(C)]
pub struct Passwd {
    name: *const c_char,
    passwd: *const c_char,
    uid: u32,
    gid: u32,
    change: i64,
    class: *const c_char,
    gecos: *const c_char,
    dir: *const c_char,
    shell: *const c_char,
    expire: i64,
}

static PW: crate::Global<Passwd> = crate::Global::new(Passwd {
    name: core::ptr::null(),
    passwd: core::ptr::null(),
    uid: 0,
    gid: 0,
    change: 0,
    class: core::ptr::null(),
    gecos: core::ptr::null(),
    dir: core::ptr::null(),
    shell: core::ptr::null(),
    expire: 0,
});

fn entry(uid: u32) -> *mut Passwd {
    // SAFETY: one use per call; the strings are static.
    unsafe {
        let pw = PW.get();
        let root = uid == 0;
        *pw = Passwd {
            name: if root { c"root".as_ptr() } else { c"user".as_ptr() },
            passwd: c"*".as_ptr(),
            uid,
            gid: getgid(),
            change: 0,
            class: c"".as_ptr(),
            gecos: if root { c"System Administrator".as_ptr() } else { c"User".as_ptr() },
            dir: if root { c"/var/root".as_ptr() } else { c"/".as_ptr() },
            shell: c"/bin/sh".as_ptr(),
            expire: 0,
        };
        pw
    }
}

#[unsafe(no_mangle)]
extern "C" fn getpwuid(uid: u32) -> *mut Passwd {
    // SAFETY: a plain call.
    if uid == unsafe { getuid() } { entry(uid) } else { core::ptr::null_mut() }
}

#[unsafe(no_mangle)]
extern "C" fn getpwnam(_name: *const c_char) -> *mut Passwd {
    // SAFETY: a plain call.
    entry(unsafe { getuid() })
}
