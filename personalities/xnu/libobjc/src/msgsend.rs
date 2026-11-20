//! Message sending (the objc_msgSend sheets).
//!
//! The entry points are assembly: they keep every argument register (and
//! `rax`, the vector-register count of a variadic call, and `xmm0` to
//! `xmm7`) intact across the lookup and jump to the method, so the method
//! runs as if the caller had called it. A nil receiver returns zero in
//! `rax`, `rdx`, `xmm0` and `xmm1` without calling anything; the `_stret`
//! variants (the result buffer first) leave the buffer alone; the
//! `_fpret` variant returns 0.0 in `st0`. The super variants take a pair
//! (receiver, class) and do not short-circuit nil.
//!
//! The lookup (`poly_objc_lookup*`) sends `+initialize` on a class's first
//! message, asks `+resolveInstanceMethod:` / `+resolveClassMethod:` once
//! for a selector the class does not have, and otherwise writes the
//! unrecognized-selector line and aborts.

use crate::{Raw, fatal, lock, sel, tid};
use alloc::vec::Vec;
use poly_objc::Mem;
use poly_objc::runtime::Init;

core::arch::global_asm!(
    ".macro POLY_SAVE",
    "    push rbp",
    "    mov rbp, rsp",
    "    sub rsp, 192",
    "    mov [rsp], rdi",
    "    mov [rsp + 8], rsi",
    "    mov [rsp + 16], rdx",
    "    mov [rsp + 24], rcx",
    "    mov [rsp + 32], r8",
    "    mov [rsp + 40], r9",
    "    mov [rsp + 48], rax",
    "    movdqu [rsp + 64], xmm0",
    "    movdqu [rsp + 80], xmm1",
    "    movdqu [rsp + 96], xmm2",
    "    movdqu [rsp + 112], xmm3",
    "    movdqu [rsp + 128], xmm4",
    "    movdqu [rsp + 144], xmm5",
    "    movdqu [rsp + 160], xmm6",
    "    movdqu [rsp + 176], xmm7",
    ".endm",
    ".macro POLY_RESTORE",
    "    movdqu xmm0, [rsp + 64]",
    "    movdqu xmm1, [rsp + 80]",
    "    movdqu xmm2, [rsp + 96]",
    "    movdqu xmm3, [rsp + 112]",
    "    movdqu xmm4, [rsp + 128]",
    "    movdqu xmm5, [rsp + 144]",
    "    movdqu xmm6, [rsp + 160]",
    "    movdqu xmm7, [rsp + 176]",
    "    mov rdi, [rsp]",
    "    mov rsi, [rsp + 8]",
    "    mov rdx, [rsp + 16]",
    "    mov rcx, [rsp + 24]",
    "    mov r8, [rsp + 32]",
    "    mov r9, [rsp + 40]",
    "    mov rax, [rsp + 48]",
    "    leave",
    ".endm",
    // objc_msgSend(self, _cmd, ...)
    ".globl _objc_msgSend",
    ".p2align 4",
    "_objc_msgSend:",
    "    test rdi, rdi",
    "    jz Lpoly_nil",
    "    POLY_SAVE",
    "    call _poly_objc_lookup",
    "    mov r11, rax",
    "    POLY_RESTORE",
    "    jmp r11",
    "Lpoly_nil:",
    "    xor eax, eax",
    "    xor edx, edx",
    "    xorps xmm0, xmm0",
    "    xorps xmm1, xmm1",
    "    ret",
    // objc_msgSend_fpret: a nil receiver returns 0.0 in st0.
    ".globl _objc_msgSend_fpret",
    ".p2align 4",
    "_objc_msgSend_fpret:",
    "    test rdi, rdi",
    "    jz Lpoly_nil_fp",
    "    jmp _objc_msgSend",
    "Lpoly_nil_fp:",
    "    fldz",
    "    xor eax, eax",
    "    xor edx, edx",
    "    ret",
    // objc_msgSend_stret(buffer, self, _cmd, ...)
    ".globl _objc_msgSend_stret",
    ".p2align 4",
    "_objc_msgSend_stret:",
    "    test rsi, rsi",
    "    jz Lpoly_nil_stret",
    "    POLY_SAVE",
    "    mov rdi, rsi",
    "    mov rsi, rdx",
    "    call _poly_objc_lookup",
    "    mov r11, rax",
    "    POLY_RESTORE",
    "    jmp r11",
    "Lpoly_nil_stret:",
    "    ret",
    // objc_msgSendSuper(&{self, class}, _cmd, ...): search from class.
    ".globl _objc_msgSendSuper",
    ".p2align 4",
    "_objc_msgSendSuper:",
    "    POLY_SAVE",
    "    call _poly_objc_lookup_super",
    "    mov r11, rax",
    "    POLY_RESTORE",
    "    mov rdi, [rdi]",
    "    jmp r11",
    // objc_msgSendSuper2(&{self, class}, _cmd, ...): search from class's
    // superclass.
    ".globl _objc_msgSendSuper2",
    ".p2align 4",
    "_objc_msgSendSuper2:",
    "    POLY_SAVE",
    "    call _poly_objc_lookup_super2",
    "    mov r11, rax",
    "    POLY_RESTORE",
    "    mov rdi, [rdi]",
    "    jmp r11",
    ".globl _objc_msgSendSuper_stret",
    ".p2align 4",
    "_objc_msgSendSuper_stret:",
    "    POLY_SAVE",
    "    mov rdi, rsi",
    "    mov rsi, rdx",
    "    call _poly_objc_lookup_super",
    "    mov r11, rax",
    "    POLY_RESTORE",
    "    mov rsi, [rsi]",
    "    jmp r11",
    ".globl _objc_msgSendSuper2_stret",
    ".p2align 4",
    "_objc_msgSendSuper2_stret:",
    "    POLY_SAVE",
    "    mov rdi, rsi",
    "    mov rsi, rdx",
    "    call _poly_objc_lookup_super2",
    "    mov r11, rax",
    "    POLY_RESTORE",
    "    mov rsi, [rsi]",
    "    jmp r11",
    // _objc_msgForward(self, _cmd, ...): the IMP of a selector a class
    // does not implement (no forwarding handler: abort).
    ".globl __objc_msgForward",
    ".p2align 4",
    "__objc_msgForward:",
    "    push rbp",
    "    mov rbp, rsp",
    "    call _poly_objc_forward",
    "    ud2",
    ".globl __objc_msgForward_stret",
    ".p2align 4",
    "__objc_msgForward_stret:",
    "    push rbp",
    "    mov rbp, rsp",
    "    mov rdi, rsi",
    "    mov rsi, rdx",
    "    call _poly_objc_forward",
    "    ud2",
);

unsafe extern "C" {
    pub fn objc_msgSend();
    pub fn _objc_msgForward();
}

/// `objc_msgSend` as a function of `(self, _cmd)`.
pub(crate) fn send0(obj: u64, sel: u64) -> u64 {
    // SAFETY: objc_msgSend takes the method's own signature.
    let f: extern "C" fn(u64, u64) -> u64 = unsafe { core::mem::transmute(objc_msgSend as *const ()) };
    f(obj, sel)
}

/// `objc_msgSend` as a function of `(self, _cmd, arg)`.
pub(crate) fn send1(obj: u64, sel: u64, a: u64) -> u64 {
    // SAFETY: as for `send0`.
    let f: extern "C" fn(u64, u64, u64) -> u64 = unsafe { core::mem::transmute(objc_msgSend as *const ()) };
    f(obj, sel, a)
}

/// `objc_msgSend` of `(self, _cmd, arg)` for a method returning `BOOL`.
pub(crate) fn send1_bool(obj: u64, sel: u64, a: u64) -> bool {
    // SAFETY: as for `send0`; BOOL is a signed char on x86-64.
    let f: extern "C" fn(u64, u64, u64) -> i8 = unsafe { core::mem::transmute(objc_msgSend as *const ()) };
    f(obj, sel, a) != 0
}

/// The class object whose metaclass is `meta` (0 if none).
fn class_of_meta(meta: u64) -> u64 {
    lock().rt.class_for_meta(meta).unwrap_or(0)
}

/// Make sure `+initialize` has been sent to `cls` (a class, not a
/// metaclass) and its superclasses, superclass first, once each.
pub(crate) fn initialize(cls: u64) {
    if cls == 0 {
        return;
    }
    let me = tid();
    loop {
        let mut g = lock();
        let Some(c) = g.rt.class_mut(cls) else { return };
        match c.init {
            Init::Done => return,
            Init::Running(t) if t == me => return,
            Init::Running(_) => {
                drop(g);
                core::hint::spin_loop();
                continue;
            }
            Init::No => {
                c.init = Init::Running(me);
                break;
            }
        }
    }
    let (sup, meta) = {
        let g = lock();
        let c = g.rt.class(cls).map(|c| (c.superclass, c.isa)).unwrap_or((0, 0));
        c
    };
    initialize(sup);
    let s = sel(b"initialize");
    let imp = lock().rt.lookup(meta, s);
    match imp {
        // SAFETY: +initialize is (self, _cmd).
        Some(imp) => unsafe { core::mem::transmute::<usize, extern "C" fn(u64, u64)>(imp as usize)(cls, s) },
        None => unrecognized(cls, s, meta),
    }
    if let Some(c) = lock().rt.class_mut(cls) {
        c.init = Init::Done;
    }
}

/// Ask the resolver of `cls` (an instance's class or a metaclass) about
/// `sel`, the first time; whether it may have added a method.
pub(crate) fn resolve(cls: u64, s: u64) -> bool {
    let (meta, target) = {
        let mut g = lock();
        if !g.rt.should_resolve(cls, s) {
            return false;
        }
        let Some(c) = g.rt.class(cls) else { return false };
        if c.meta {
            let k = g.rt.class_for_meta(cls).unwrap_or(0);
            (cls, k)
        } else {
            (c.isa, cls)
        }
    };
    let is_meta = meta == cls;
    let resolver = sel(if is_meta { b"resolveClassMethod:" } else { b"resolveInstanceMethod:" });
    let imp = lock().rt.lookup(meta, resolver);
    let Some(imp) = imp else { return false };
    // SAFETY: +resolve...Method: is (self, _cmd, SEL) -> BOOL.
    let f: extern "C" fn(u64, u64, u64) -> i8 = unsafe { core::mem::transmute(imp as usize) };
    f(target, resolver, s) != 0
}

/// The IMP for `s` sent to `receiver`, searching from `cls`.
fn lookup_imp(cls: u64, s: u64, receiver: u64, init: bool) -> u64 {
    if init {
        let is_meta = lock().rt.class(cls).is_some_and(|c| c.meta);
        initialize(if is_meta { class_of_meta(cls) } else { cls });
    }
    loop {
        if let Some(imp) = lock().rt.lookup(cls, s) {
            return imp;
        }
        if !resolve(cls, s) {
            if let Some(imp) = lock().rt.lookup(cls, s) {
                return imp;
            }
            unrecognized(cls, s, receiver);
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn poly_objc_lookup(obj: u64, s: u64) -> u64 {
    lookup_imp(Raw.u64(obj), s, obj, true)
}

#[unsafe(no_mangle)]
extern "C" fn poly_objc_lookup_super(pair: u64, s: u64) -> u64 {
    lookup_imp(Raw.u64(pair + 8), s, Raw.u64(pair), true)
}

#[unsafe(no_mangle)]
extern "C" fn poly_objc_lookup_super2(pair: u64, s: u64) -> u64 {
    let cls = Raw.u64(pair + 8);
    let sup = lock().rt.class(cls).map_or(0, |c| c.superclass);
    lookup_imp(sup, s, Raw.u64(pair), true)
}

#[unsafe(no_mangle)]
extern "C" fn poly_objc_forward(obj: u64, s: u64) -> ! {
    let cls = Raw.u64(obj);
    unrecognized(cls, s, obj)
}

/// Append `n` in decimal.
pub(crate) fn push_dec(out: &mut Vec<u8>, mut n: u64) {
    let mut b = [0u8; 20];
    let mut i = b.len();
    loop {
        i -= 1;
        b[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    out.extend_from_slice(&b[i..]);
}

/// Append `n` as 0x and lowercase hex.
pub(crate) fn push_hex(out: &mut Vec<u8>, n: u64) {
    out.extend_from_slice(b"0x");
    let digits = b"0123456789abcdef";
    let mut started = false;
    for i in (0..16).rev() {
        let d = ((n >> (i * 4)) & 15) as usize;
        if d != 0 || started || i == 0 {
            started = true;
            out.push(digits[d]);
        }
    }
}

/// "objc[pid]: -[Class sel]: unrecognized selector sent to instance
/// 0xADDR (no message forward handler is installed)", then SIGABRT.
pub(crate) fn unrecognized(cls: u64, s: u64, receiver: u64) -> ! {
    let (meta, name) = {
        let g = lock();
        let c = g.rt.class(cls);
        (c.is_some_and(|c| c.meta), c.map_or_else(Vec::new, |c| Raw.cstr(c.name)))
    };
    let mut m = Vec::new();
    m.extend_from_slice(b"objc[");
    // SAFETY: getpid has no preconditions.
    push_dec(&mut m, unsafe { crate::sys::getpid() } as u64);
    m.extend_from_slice(b"]: ");
    m.push(if meta { b'+' } else { b'-' });
    m.push(b'[');
    m.extend_from_slice(&name);
    m.push(b' ');
    m.extend_from_slice(&Raw.cstr(s));
    m.extend_from_slice(if meta {
        b"]: unrecognized selector sent to class "
    } else {
        b"]: unrecognized selector sent to instance "
    });
    push_hex(&mut m, receiver);
    m.extend_from_slice(b" (no message forward handler is installed)\n");
    fatal(&m)
}
