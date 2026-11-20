//! Memory management (the objc_retain ... objc_storeWeak,
//! objc_setAssociatedObject, objc_sync_* and exception sheets).
//!
//! The entry points the compiler emits send the messages (`-retain`,
//! `-release`, `-autorelease`), so overrides see one call each; NSObject's
//! own methods keep the counts in `poly_objc::arc`. Class objects are never
//! counted.
//!
//! The return-value handshake: `objc_autoreleaseReturnValue` looks at the
//! code it returns to; when that is `mov %rax, %rdi` and a call that
//! reaches `objc_retainAutoreleasedReturnValue` (through a stub or the GOT),
//! the object is handed over unpooled and the caller's call does not
//! retain it.

use crate::msgsend::{push_hex, send0, send1, send1_bool};
use crate::{Raw, fatal, lock, sel, tid};
use alloc::vec::Vec;
use poly_objc::Mem;
use poly_objc::arc::{Assoc, Released, policy};

// ---- NSObject's own counting ------------------------------------------------

/// `-[NSObject retain]`.
pub(crate) fn retain_obj(obj: u64) -> u64 {
    let mut g = lock();
    if g.rt.class(obj).is_none() {
        g.refs.retain(obj);
    }
    obj
}

/// `-[NSObject release]`: `-dealloc` when the last reference goes.
pub(crate) fn release_obj(obj: u64) {
    let last = {
        let mut g = lock();
        g.rt.class(obj).is_none() && g.refs.release(obj) == Released::Dealloc
    };
    if last {
        send0(obj, sel(b"dealloc"));
    }
}

/// `-[NSObject autorelease]`: into the thread's innermost pool, or leaked.
pub(crate) fn autorelease_obj(obj: u64) -> u64 {
    let t = tid();
    let mut g = lock();
    if g.rt.class(obj).is_none() {
        g.pools.entry(t).or_default().add(obj);
    }
    obj
}

// ---- the compiler's entry points ---------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn objc_retain(obj: u64) -> u64 {
    if obj == 0 { 0 } else { send0(obj, sel(b"retain")) }
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_release(obj: u64) {
    if obj != 0 {
        send0(obj, sel(b"release"));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_autorelease(obj: u64) -> u64 {
    if obj == 0 { 0 } else { send0(obj, sel(b"autorelease")) }
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_retainAutorelease(obj: u64) -> u64 {
    objc_autorelease(objc_retain(obj))
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_storeStrong(location: *mut u64, new: u64) {
    // SAFETY: the caller's strong variable.
    let old = unsafe { *location };
    if old == new {
        return;
    }
    objc_retain(new);
    // SAFETY: as above.
    unsafe { *location = new };
    objc_release(old);
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_autoreleasePoolPush() -> u64 {
    let t = tid();
    lock().pools.entry(t).or_default().push()
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_autoreleasePoolPop(token: u64) {
    if token == 0 {
        // A null token is not a pool: the process dies by SIGSEGV, as the
        // sheet observed.
        // SAFETY: a deliberate fault at address 0.
        unsafe { core::arch::asm!("mov al, byte ptr [0]", out("al") _) };
    }
    let t = tid();
    let Some(start) = lock().pools.get(&t).and_then(|p| p.start_of(token)) else { return };
    loop {
        let next = lock().pools.get_mut(&t).and_then(|p| p.pop_one(start));
        match next {
            Some(obj) => objc_release(obj),
            None => break,
        }
    }
}

// ---- the return-value handshake ------------------------------------------------

core::arch::global_asm!(
    ".globl _objc_autoreleaseReturnValue",
    ".p2align 4",
    "_objc_autoreleaseReturnValue:",
    "    mov rsi, [rsp]",
    "    jmp _poly_objc_autorelease_rv",
    ".globl _objc_retainAutoreleaseReturnValue",
    ".p2align 4",
    "_objc_retainAutoreleaseReturnValue:",
    "    mov rsi, [rsp]",
    "    jmp _poly_objc_retain_autorelease_rv",
);

fn rel32(at: u64) -> u64 {
    Raw.u32(at) as i32 as i64 as u64
}

/// Whether the code at `ra` (where the callee returns to) hands the result
/// straight to `objc_retainAutoreleasedReturnValue` or one of its claim
/// siblings: `mov %rax, %rdi`, then `call` of a stub (`jmp *slot`) or
/// `call *slot`, the slot holding one of those functions.
fn paired(ra: u64) -> bool {
    if ra == 0 || [Raw.u8(ra), Raw.u8(ra + 1), Raw.u8(ra + 2)] != [0x48, 0x89, 0xc7] {
        return false;
    }
    let call = ra + 3;
    let target = match (Raw.u8(call), Raw.u8(call + 1)) {
        (0xe8, _) => {
            let stub = call.wrapping_add(5).wrapping_add(rel32(call + 1));
            if (Raw.u8(stub), Raw.u8(stub + 1)) == (0xff, 0x25) {
                Raw.u64(stub.wrapping_add(6).wrapping_add(rel32(stub + 2)))
            } else {
                stub
            }
        }
        (0xff, 0x15) => Raw.u64(call.wrapping_add(6).wrapping_add(rel32(call + 2))),
        _ => return false,
    };
    target == objc_retainAutoreleasedReturnValue as *const () as u64
        || target == objc_claimAutoreleasedReturnValue as *const () as u64
        || target == objc_unsafeClaimAutoreleasedReturnValue as *const () as u64
}

#[unsafe(no_mangle)]
extern "C" fn poly_objc_autorelease_rv(obj: u64, ra: u64) -> u64 {
    if obj == 0 {
        return 0;
    }
    if paired(ra) {
        let t = tid();
        lock().handoff.insert(t, obj);
        return obj;
    }
    objc_autorelease(obj)
}

#[unsafe(no_mangle)]
extern "C" fn poly_objc_retain_autorelease_rv(obj: u64, ra: u64) -> u64 {
    if obj == 0 {
        return 0;
    }
    objc_retain(obj);
    poly_objc_autorelease_rv(obj, ra)
}

/// Whether `obj` was handed over unpooled to this thread just now.
fn take_handoff(obj: u64) -> bool {
    let t = tid();
    lock().handoff.remove(&t) == Some(obj)
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_retainAutoreleasedReturnValue(obj: u64) -> u64 {
    if obj == 0 {
        return 0;
    }
    if take_handoff(obj) { obj } else { objc_retain(obj) }
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_claimAutoreleasedReturnValue(obj: u64) -> u64 {
    objc_retainAutoreleasedReturnValue(obj)
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_unsafeClaimAutoreleasedReturnValue(obj: u64) -> u64 {
    if obj != 0 && take_handoff(obj) {
        objc_release(obj);
    }
    obj
}

// ---- weak references --------------------------------------------------------

fn weak_target_check(obj: u64) {
    if lock().refs.is_deallocating(obj) {
        let mut m = b"objc[".to_vec();
        // SAFETY: getpid has no preconditions.
        crate::msgsend::push_dec(&mut m, unsafe { crate::sys::getpid() } as u64);
        m.extend_from_slice(b"]: Cannot form weak reference to instance (");
        push_hex(&mut m, obj);
        m.extend_from_slice(
            b"). It is possible that this object was over-released, or is in the process of deallocation.\n",
        );
        fatal(&m);
    }
    send0(obj, sel(b"allowsWeakReference"));
}

fn store_weak(location: u64, obj: u64) -> u64 {
    if obj != 0 {
        weak_target_check(obj);
    }
    lock().weak.store(location, obj);
    Raw.write_u64(location, obj);
    obj
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_storeWeak(location: *mut u64, obj: u64) -> u64 {
    store_weak(location as u64, obj)
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_initWeak(location: *mut u64, obj: u64) -> u64 {
    store_weak(location as u64, obj)
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_initWeakOrNil(location: *mut u64, obj: u64) -> u64 {
    let ok = obj != 0 && !lock().refs.is_deallocating(obj);
    store_weak(location as u64, if ok { obj } else { 0 })
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_loadWeakRetained(location: *mut u64) -> u64 {
    let obj = {
        let g = lock();
        match g.weak.target(location as u64) {
            Some(o) if !g.refs.is_deallocating(o) => o,
            _ => return 0,
        }
    };
    if !send1_bool_noarg(obj, sel(b"retainWeakReference")) {
        return 0;
    }
    objc_retain(obj)
}

fn send1_bool_noarg(obj: u64, s: u64) -> bool {
    // SAFETY: a BOOL method of (self, _cmd).
    let f: extern "C" fn(u64, u64) -> i8 = unsafe { core::mem::transmute(crate::msgsend::objc_msgSend as *const ()) };
    f(obj, s) != 0
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_loadWeak(location: *mut u64) -> u64 {
    objc_autorelease(objc_loadWeakRetained(location))
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_destroyWeak(location: *mut u64) {
    lock().weak.unregister(location as u64);
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_moveWeak(to: *mut u64, from: *mut u64) {
    let obj = {
        let mut g = lock();
        let o = g.weak.target(from as u64).unwrap_or(0);
        g.weak.unregister(from as u64);
        g.weak.store(to as u64, o);
        o
    };
    Raw.write_u64(from as u64, 0);
    Raw.write_u64(to as u64, obj);
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_copyWeak(to: *mut u64, from: *mut u64) {
    let obj = {
        let mut g = lock();
        let o = match g.weak.target(from as u64) {
            Some(o) if !g.refs.is_deallocating(o) => o,
            _ => 0,
        };
        g.weak.store(to as u64, o);
        o
    };
    Raw.write_u64(to as u64, obj);
}

// ---- associated objects --------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn objc_setAssociatedObject(obj: u64, key: u64, value: u64, pol: u64) {
    if obj == 0 {
        if value == 0 {
            return;
        }
        // A value on a nil object: SIGSEGV, as the sheet observed.
        // SAFETY: a deliberate fault at address 0.
        unsafe { core::arch::asm!("mov al, byte ptr [0]", out("al") _) };
    }
    let stored = if value == 0 {
        0
    } else if policy::copies(pol) {
        send1(value, sel(b"copyWithZone:"), 0)
    } else if policy::owns(pol) {
        objc_retain(value)
    } else {
        value
    };
    let old = lock().assoc.set(obj, key, Assoc { value: stored, policy: pol });
    if let Some(o) = old
        && policy::owns(o.policy)
    {
        objc_release(o.value);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_getAssociatedObject(obj: u64, key: u64) -> u64 {
    if obj == 0 {
        return 0;
    }
    let Some(a) = lock().assoc.get(obj, key) else { return 0 };
    if policy::atomic(a.policy) { objc_autorelease(objc_retain(a.value)) } else { a.value }
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_removeAssociatedObjects(obj: u64) {
    if obj == 0 {
        return;
    }
    let all = lock().assoc.remove_all(obj);
    for a in all {
        if policy::owns(a.policy) {
            objc_release(a.value);
        }
    }
}

// ---- @synchronized ---------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn objc_sync_enter(obj: u64) -> i32 {
    if obj == 0 {
        return 0;
    }
    let t = tid();
    while !lock().sync.try_enter(obj, t) {
        core::hint::spin_loop();
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_sync_exit(obj: u64) -> i32 {
    if obj == 0 {
        return 0;
    }
    let t = tid();
    if lock().sync.exit(obj, t) { 0 } else { -1 }
}

// ---- exceptions ---------------------------------------------------------------------

/// `objc_exception_throw`. v0 has no unwinder: every exception is
/// uncaught, which ends the process the way the sheet records it.
#[unsafe(no_mangle)]
pub extern "C" fn objc_exception_throw(obj: u64) -> ! {
    let name: Vec<u8> = if obj == 0 {
        b"nil".to_vec()
    } else {
        let g = lock();
        // A class object's name is its own; an instance's its class's.
        let cls = if g.rt.class(obj).is_some() { obj } else { Raw.u64(obj) };
        g.rt.class(cls).map_or_else(|| b"?".to_vec(), |c| Raw.cstr(c.name))
    };
    let mut m = b"libc++abi: terminating due to uncaught exception of type ".to_vec();
    m.extend_from_slice(&name);
    m.push(b'\n');
    fatal(&m)
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_exception_rethrow() -> ! {
    fatal(b"libc++abi: terminating due to uncaught exception\n")
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_begin_catch(exception: u64) -> u64 {
    exception
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_end_catch() {}

#[unsafe(no_mangle)]
pub extern "C" fn objc_terminate() -> ! {
    fatal(b"libc++abi: terminating\n")
}

/// The personality routine of Objective-C exception tables: no handler is
/// ever found by v0 (`_URC_CONTINUE_UNWIND`).
#[unsafe(export_name = "__objc_personality_v0")]
pub extern "C" fn objc_personality_v0(_v: i32, _actions: i32, _class: u64, _ex: u64, _ctx: u64) -> i32 {
    8
}

/// The exception type of `@catch (id)` clauses.
#[unsafe(export_name = "OBJC_EHTYPE_id")]
pub static EHTYPE_ID: crate::nsobject::Shared<[u64; 3]> = crate::nsobject::Shared([0; 3]);

#[allow(dead_code)]
fn _uses() {
    let _ = send1_bool;
}
