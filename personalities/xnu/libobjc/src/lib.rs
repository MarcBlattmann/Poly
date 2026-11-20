//! Poly's own `libobjc.A.dylib` (ADR 0259): the Objective-C runtime that
//! programs compiled against the macOS SDK load in place of Apple's. It is
//! built for `x86_64-apple-darwin` and linked against Poly's libSystem
//! (ADR 0258) by `cargo xtask libobjc`; the export list is in `exports/`.
//!
//! - [`msgsend`]: `objc_msgSend` and its variants (assembly), method lookup
//!   with `+initialize` and the resolvers, the unrecognized-selector abort.
//! - [`nsobject`]: the root class `NSObject`, its class objects in the
//!   compiler's layout and its methods.
//! - [`api`]: the class, method, selector, protocol and object functions of
//!   `<objc/runtime.h>`.
//! - [`memory`]: reference counting entry points, autorelease pools, weak
//!   references, associated objects, `@synchronized`, exceptions.
//!
//! The tables and their logic are `poly_objc`'s; this crate holds them
//! behind one lock, reaches memory through raw pointers, and makes the
//! calls into the program (`+load`, `+initialize`, resolvers, `-dealloc`)
//! with the lock released. At start (an initializer, which Poly's dyld runs
//! after libSystem's), it maps every image dyld has loaded.
//!
//! Inputs: the spec sheets `specs/macos/libobjc.A.dylib`, the SDK headers
//! `<objc/runtime.h>` and `<objc/message.h>`, the export names of the SDK's
//! `libobjc.tbd`, and the compiler's output for our own sources (ADR 0259).
//! No part of Apple's runtime was read or disassembled.

#![no_std]

extern crate alloc;

mod api;
mod memory;
mod msgsend;
mod nsobject;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_objc::Mem;
use poly_objc::arc::{Associations, Pools, RefCounts, SyncLocks, WeakTable};
use poly_objc::runtime::Runtime;

/// libSystem (ADR 0258).
#[allow(dead_code)]
mod sys {
    use core::ffi::{c_char, c_void};
    unsafe extern "C" {
        pub fn malloc(n: usize) -> *mut c_void;
        pub fn calloc(n: usize, size: usize) -> *mut c_void;
        pub fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        pub fn free(p: *mut c_void);
        pub fn write(fd: i32, buf: *const c_void, n: usize) -> isize;
        pub fn abort() -> !;
        pub fn getpid() -> i32;
        pub fn _dyld_image_count() -> u32;
        pub fn _dyld_get_image_header(i: u32) -> *const c_void;
        pub fn _dyld_get_image_name(i: u32) -> *const c_char;
    }
}

/// Memory from libSystem's malloc for Rust's collections.
struct Malloc;

// SAFETY: malloc, realloc and free are the C allocator; 16-byte alignment
// (macOS's malloc) covers every layout this crate allocates, and larger
// alignments are refused.
unsafe impl core::alloc::GlobalAlloc for Malloc {
    unsafe fn alloc(&self, l: core::alloc::Layout) -> *mut u8 {
        if l.align() > 16 {
            return core::ptr::null_mut();
        }
        // SAFETY: plain C allocation.
        unsafe { sys::malloc(l.size().max(1)).cast() }
    }
    unsafe fn dealloc(&self, p: *mut u8, _l: core::alloc::Layout) {
        // SAFETY: `p` came from malloc or realloc.
        unsafe { sys::free(p.cast()) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: core::alloc::Layout, n: usize) -> *mut u8 {
        if l.align() > 16 {
            return core::ptr::null_mut();
        }
        // SAFETY: `p` came from malloc or realloc.
        unsafe { sys::realloc(p.cast(), n.max(1)).cast() }
    }
}

#[global_allocator]
static ALLOC: Malloc = Malloc;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    write_err(b"objc[poly]: internal error\n");
    // SAFETY: abort never returns.
    unsafe { sys::abort() }
}

/// Write bytes to standard error.
pub(crate) fn write_err(b: &[u8]) {
    // SAFETY: a plain write of a valid buffer.
    unsafe { sys::write(2, b.as_ptr().cast(), b.len()) };
}

/// Write `msg` to standard error and end the process with SIGABRT.
pub(crate) fn fatal(msg: &[u8]) -> ! {
    write_err(msg);
    // SAFETY: abort never returns.
    unsafe { sys::abort() }
}

/// The process's memory, through raw pointers. Every address the runtime
/// reads is one the program or dyld gave it (metadata, objects, names).
pub(crate) struct Raw;

impl Mem for Raw {
    fn u8(&self, addr: u64) -> u8 {
        if addr == 0 {
            return 0;
        }
        // SAFETY: see the type's documentation.
        unsafe { core::ptr::read(addr as *const u8) }
    }
    fn u32(&self, addr: u64) -> u32 {
        // SAFETY: see the type's documentation.
        unsafe { core::ptr::read_unaligned(addr as *const u32) }
    }
    fn u64(&self, addr: u64) -> u64 {
        if addr == 0 {
            return 0;
        }
        // SAFETY: see the type's documentation.
        unsafe { core::ptr::read_unaligned(addr as *const u64) }
    }
    fn write_u64(&mut self, addr: u64, value: u64) {
        // SAFETY: selector references and class objects the runtime made:
        // writable memory.
        unsafe { core::ptr::write_unaligned(addr as *mut u64, value) }
    }
    fn alloc(&mut self, bytes: &[u8]) -> u64 {
        // SAFETY: a fresh allocation of the right size, never freed.
        unsafe {
            let p: *mut u8 = sys::malloc(bytes.len().max(1)).cast();
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
            p as u64
        }
    }
    fn cstr(&self, addr: u64) -> Vec<u8> {
        let mut out = Vec::new();
        if addr == 0 {
            return out;
        }
        // SAFETY: a NUL-terminated string the program or the runtime made.
        unsafe {
            let mut p = addr as *const u8;
            while *p != 0 && out.len() < 1 << 16 {
                out.push(*p);
                p = p.add(1);
            }
        }
        out
    }
}

/// Everything the runtime keeps.
pub(crate) struct State {
    pub rt: Runtime,
    pub refs: RefCounts,
    pub weak: WeakTable,
    pub assoc: Associations,
    /// Autorelease pools, per thread.
    pub pools: alloc::collections::BTreeMap<u64, Pools>,
    pub sync: SyncLocks,
    /// The object an `objc_autoreleaseReturnValue` handed over unpooled,
    /// per thread (the return-value handshake).
    pub handoff: alloc::collections::BTreeMap<u64, u64>,
}

static LOCKED: AtomicBool = AtomicBool::new(false);
static mut STATE: Option<State> = None;

/// The runtime's state, locked until the guard goes. Never held across a
/// call into the program.
pub(crate) struct Guard;

impl core::ops::Deref for Guard {
    type Target = State;
    fn deref(&self) -> &State {
        // SAFETY: the guard holds the lock; `lock` made the state.
        unsafe { (*(&raw const STATE)).as_ref().unwrap_unchecked() }
    }
}

impl core::ops::DerefMut for Guard {
    fn deref_mut(&mut self) -> &mut State {
        // SAFETY: as for `deref`; the lock makes this the only reference.
        unsafe { (*(&raw mut STATE)).as_mut().unwrap_unchecked() }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        LOCKED.store(false, Ordering::Release);
    }
}

/// Take the runtime's lock.
pub(crate) fn lock() -> Guard {
    while LOCKED.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: the lock is held.
    unsafe {
        let s = &mut *(&raw mut STATE);
        if s.is_none() {
            *s = Some(State {
                rt: Runtime::new(),
                refs: RefCounts::default(),
                weak: WeakTable::default(),
                assoc: Associations::default(),
                pools: alloc::collections::BTreeMap::new(),
                sync: SyncLocks::default(),
                handoff: alloc::collections::BTreeMap::new(),
            });
        }
    }
    Guard
}

/// The calling thread's id (`thread_selfid`, BSD 372: the call is raw so
/// it works before libSystem has started any thread machinery).
pub(crate) fn tid() -> u64 {
    let r: u64;
    // SAFETY: a system call without arguments that only returns a value.
    unsafe {
        core::arch::asm!("syscall", inlateout("rax") 0x0200_0000u64 | 372 => r,
            out("rcx") _, out("r11") _, out("rdx") _, options(nostack));
    }
    r
}

/// The selector named `name`.
pub(crate) fn sel(name: &[u8]) -> u64 {
    lock().rt.sel(&mut Raw, name)
}

/// Map every image dyld loaded (libraries before the program: the reverse
/// of the load order; this library's own image brings NSObject), then run
/// their `+load` methods, image by image.
fn start() {
    // SAFETY: libdyld's image table (ADR 0258).
    let count = unsafe { sys::_dyld_image_count() };
    let load = sel(b"load");
    for i in (0..count).rev() {
        // SAFETY: as above.
        let (header, name) = unsafe { (sys::_dyld_get_image_header(i) as u64, sys::_dyld_get_image_name(i) as u64) };
        if header == 0 {
            continue;
        }
        let plan = lock().rt.map_image(&mut Raw, header, name).0;
        for (cls, imp) in plan {
            // SAFETY: a +load method: (self, _cmd).
            let f: extern "C" fn(u64, u64) = unsafe { core::mem::transmute(imp as usize) };
            f(cls, load);
        }
    }
}

/// The initializer dyld runs for this library.
extern "C" fn init(_argc: i32, _argv: *const u8, _envp: *const u8, _apple: *const u8, _start: *const u8) {
    start();
}

#[used]
#[unsafe(link_section = "__DATA,__mod_init_func,mod_init_funcs")]
static INIT: extern "C" fn(i32, *const u8, *const u8, *const u8, *const u8) = init;
