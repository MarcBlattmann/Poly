//! CFAllocator (`<CoreFoundation/CFBase.h>`): the standard allocators and
//! allocators made from a `CFAllocatorContext`.

use crate::base::{self, CFIndex, Header, Ref, Shared, id};
use core::ffi::c_void;

type RetainFn = Option<unsafe extern "C" fn(*const c_void) -> *const c_void>;
type ReleaseFn = Option<unsafe extern "C" fn(*const c_void)>;
type DescFn = Option<unsafe extern "C" fn(*const c_void) -> Ref>;
type AllocFn = Option<unsafe extern "C" fn(CFIndex, u64, *mut c_void) -> *mut c_void>;
type ReallocFn = Option<unsafe extern "C" fn(*mut c_void, CFIndex, u64, *mut c_void) -> *mut c_void>;
type DeallocFn = Option<unsafe extern "C" fn(*mut c_void, *mut c_void)>;
type SizeFn = Option<unsafe extern "C" fn(CFIndex, u64, *mut c_void) -> CFIndex>;

/// `CFAllocatorContext` (`<CoreFoundation/CFBase.h>`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Context {
    pub version: CFIndex,
    pub info: *mut c_void,
    pub retain: RetainFn,
    pub release: ReleaseFn,
    pub copy_description: DescFn,
    pub allocate: AllocFn,
    pub reallocate: ReallocFn,
    pub deallocate: DeallocFn,
    pub preferred_size: SizeFn,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// malloc and free (`kCFAllocatorSystemDefault`, `kCFAllocatorMalloc`,
    /// `kCFAllocatorMallocZone`).
    Malloc,
    /// Allocates nothing, frees nothing (`kCFAllocatorNull`).
    Null,
    /// The marker `kCFAllocatorUseContext`.
    UseContext,
    Custom,
}

#[repr(C)]
pub struct Allocator {
    h: Header,
    kind: Kind,
    ctx: Context,
}

// SAFETY: the static allocators are never written.
unsafe impl Sync for Allocator {}

const NO_CONTEXT: Context = Context {
    version: 0,
    info: core::ptr::null_mut(),
    retain: None,
    release: None,
    copy_description: None,
    allocate: None,
    reallocate: None,
    deallocate: None,
    preferred_size: None,
};

const fn standard(kind: Kind) -> Allocator {
    Allocator { h: Header::immortal(&base::ALLOCATOR_CLASS, id::ALLOCATOR), kind, ctx: NO_CONTEXT }
}

static SYSTEM_DEFAULT: Allocator = standard(Kind::Malloc);
static MALLOC: Allocator = standard(Kind::Malloc);
static MALLOC_ZONE: Allocator = standard(Kind::Malloc);
static NULL: Allocator = standard(Kind::Null);
static USE_CONTEXT: Allocator = standard(Kind::UseContext);

#[unsafe(no_mangle)]
pub static kCFAllocatorDefault: Shared<c_void> = Shared(core::ptr::null());
#[unsafe(no_mangle)]
pub static kCFAllocatorSystemDefault: Shared<Allocator> = Shared(&SYSTEM_DEFAULT);
#[unsafe(no_mangle)]
pub static kCFAllocatorMalloc: Shared<Allocator> = Shared(&MALLOC);
#[unsafe(no_mangle)]
pub static kCFAllocatorMallocZone: Shared<Allocator> = Shared(&MALLOC_ZONE);
#[unsafe(no_mangle)]
pub static kCFAllocatorNull: Shared<Allocator> = Shared(&NULL);
#[unsafe(no_mangle)]
pub static kCFAllocatorUseContext: Shared<Allocator> = Shared(&USE_CONTEXT);

static DEFAULT: crate::Global<Ref> = crate::Global::new(core::ptr::null());

pub fn system_default() -> Ref {
    &SYSTEM_DEFAULT as *const Allocator as Ref
}

pub fn null_allocator() -> Ref {
    &NULL as *const Allocator as Ref
}

/// The allocator `a` stands for: null is the current default.
pub fn resolve(a: Ref) -> Ref {
    if !a.is_null() {
        return a;
    }
    // SAFETY: one thread (v0).
    let d = unsafe { *DEFAULT.get() };
    if d.is_null() { system_default() } else { d }
}

fn get<'a>(a: Ref) -> &'a Allocator {
    // SAFETY: an allocator object (the API's contract).
    unsafe { &*(resolve(a) as *const Allocator) }
}

/// What an object keeps of its allocator: null for the standard ones,
/// otherwise the allocator, retained.
pub fn retain_for_object(a: Ref) -> Ref {
    let al = get(a);
    if al.kind == Kind::Custom {
        base::retain(a);
        a
    } else {
        core::ptr::null()
    }
}

pub fn release_for_object(a: Ref) {
    if !a.is_null() {
        base::release(a);
    }
}

pub fn allocate(a: Ref, size: usize) -> *mut c_void {
    let al = get(a);
    match al.kind {
        // SAFETY: plain C call.
        Kind::Malloc | Kind::UseContext => unsafe { crate::sys::malloc(size.max(1)) },
        Kind::Null => core::ptr::null_mut(),
        // SAFETY: the context's function, as the program gave it.
        Kind::Custom => match al.ctx.allocate {
            Some(f) => unsafe { f(size as CFIndex, 0, al.ctx.info) },
            None => core::ptr::null_mut(),
        },
    }
}

pub fn deallocate(a: Ref, p: *mut c_void) {
    if p.is_null() {
        return;
    }
    let al = get(a);
    match al.kind {
        // SAFETY: `p` came from malloc.
        Kind::Malloc | Kind::UseContext => unsafe { crate::sys::free(p) },
        Kind::Null => {}
        Kind::Custom => {
            if let Some(f) = al.ctx.deallocate {
                // SAFETY: the context's function with memory it gave.
                unsafe { f(p, al.ctx.info) }
            }
        }
    }
}

pub fn reallocate(a: Ref, p: *mut c_void, size: usize) -> *mut c_void {
    let al = get(a);
    match al.kind {
        // SAFETY: `p` is null or came from malloc.
        Kind::Malloc | Kind::UseContext => unsafe { crate::sys::realloc(p, size.max(1)) },
        Kind::Null => core::ptr::null_mut(),
        // SAFETY: the context's function.
        Kind::Custom => match (p.is_null(), al.ctx.reallocate) {
            (true, _) => allocate(a, size),
            (false, Some(f)) => unsafe { f(p, size as CFIndex, 0, al.ctx.info) },
            (false, None) => core::ptr::null_mut(),
        },
    }
}

/// # Safety
/// The count of the allocator object reached zero.
pub unsafe fn destroy(cf: Ref) {
    // SAFETY: an allocator made by CFAllocatorCreate.
    let al = unsafe { &*(cf as *const Allocator) };
    if let Some(r) = al.ctx.release {
        // SAFETY: the context's function with its own info.
        unsafe { r(al.ctx.info) };
    }
    if al.h.info & base::IMMORTAL != 0 {
        return;
    }
    // SAFETY: made by `base::make` with the system allocator.
    unsafe { base::destroy::<Allocator>(cf) }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFAllocatorGetTypeID() -> usize {
    id::ALLOCATOR
}

#[unsafe(no_mangle)]
pub extern "C" fn CFAllocatorGetDefault() -> Ref {
    resolve(core::ptr::null())
}

#[unsafe(no_mangle)]
pub extern "C" fn CFAllocatorSetDefault(a: Ref) {
    if a.is_null() {
        return;
    }
    base::retain(a);
    // SAFETY: one thread (v0).
    unsafe { *DEFAULT.get() = a };
}

/// # Safety
/// `ctx` points to a `CFAllocatorContext`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFAllocatorCreate(alloc: Ref, ctx: *const Context) -> Ref {
    if ctx.is_null() {
        return core::ptr::null();
    }
    // SAFETY: the caller's contract.
    let mut c = unsafe { *ctx };
    if let Some(r) = c.retain {
        // SAFETY: the context's own function.
        c.info = unsafe { r(c.info) } as *mut c_void;
    }
    let from = if alloc == (&USE_CONTEXT as *const Allocator as Ref) { system_default() } else { alloc };
    base::make(from, id::ALLOCATOR, false, |h| Allocator { h, kind: Kind::Custom, ctx: c }) as Ref
}

#[unsafe(no_mangle)]
pub extern "C" fn CFAllocatorAllocate(a: Ref, size: CFIndex, _hint: u64) -> *mut c_void {
    if size <= 0 {
        return core::ptr::null_mut();
    }
    allocate(resolve(a), size as usize)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFAllocatorReallocate(a: Ref, p: *mut c_void, size: CFIndex, _hint: u64) -> *mut c_void {
    let a = resolve(a);
    if size <= 0 {
        deallocate(a, p);
        return core::ptr::null_mut();
    }
    reallocate(a, p, size as usize)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFAllocatorDeallocate(a: Ref, p: *mut c_void) {
    deallocate(resolve(a), p)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFAllocatorGetPreferredSizeForSize(a: Ref, size: CFIndex, hint: u64) -> CFIndex {
    let al = get(a);
    match al.ctx.preferred_size {
        // SAFETY: the context's function.
        Some(f) if al.kind == Kind::Custom => unsafe { f(size, hint, al.ctx.info) },
        _ => size,
    }
}

/// # Safety
/// `ctx` points to a `CFAllocatorContext` to fill.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFAllocatorGetContext(a: Ref, ctx: *mut Context) {
    if !ctx.is_null() {
        // SAFETY: the caller's contract.
        unsafe { *ctx = get(a).ctx };
    }
}
