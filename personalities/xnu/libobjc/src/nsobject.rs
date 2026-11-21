//! The root class `NSObject`: its class and metaclass objects in the layout
//! the compiler uses for every class (`poly_objc::abi`), exported as
//! `_OBJC_CLASS_$_NSObject` and `_OBJC_METACLASS_$_NSObject`, listed in this
//! library's `__objc_classlist` so the runtime maps it like any compiled
//! class, and its methods. Type encodings are the sheets' (x86-64: BOOL is
//! `c`).

use crate::memory::{autorelease_obj, release_obj, retain_obj};
use crate::msgsend::{send0, send1, unrecognized};
use crate::{Raw, api, lock, sel};
use poly_objc::Mem;

#[repr(C)]
pub struct ClassObj {
    isa: *const ClassObj,
    superclass: *const ClassObj,
    cache: *const u64,
    vtable: usize,
    data: *const Ro,
}

#[repr(C)]
pub struct Ro {
    flags: u32,
    instance_start: u32,
    instance_size: u32,
    reserved: u32,
    ivar_layout: usize,
    name: *const u8,
    methods: *const u8,
    protocols: usize,
    ivars: usize,
    weak_ivar_layout: usize,
    properties: usize,
}

#[repr(C)]
struct Entry {
    name: *const u8,
    types: *const u8,
    imp: *const (),
}

#[repr(C)]
struct List<const N: usize> {
    entsize: u32,
    count: u32,
    entries: [Entry; N],
}

/// Read-only data made of pointers into this library.
#[repr(transparent)]
pub struct Shared<T>(pub T);
// SAFETY: written once by the linker and dyld, read-only afterwards.
unsafe impl<T> Sync for Shared<T> {}

macro_rules! m {
    ($name:literal, $types:literal, $f:expr) => {
        Entry { name: concat!($name, "\0").as_ptr(), types: concat!($types, "\0").as_ptr(), imp: $f as *const () }
    };
}

static NAME: &[u8] = b"NSObject\0";

/// The empty method cache every compiled class points to.
#[unsafe(export_name = "_objc_empty_cache")]
pub static EMPTY_CACHE: Shared<[u64; 2]> = Shared([0; 2]);
/// The empty vtable of older compilers.
#[unsafe(export_name = "_objc_empty_vtable")]
pub static EMPTY_VTABLE: Shared<[u64; 2]> = Shared([0; 2]);

static INSTANCE_METHODS: Shared<List<29>> = Shared(List {
    entsize: 24,
    count: 29,
    entries: [
        m!("init", "@16@0:8", i_self),
        m!("dealloc", "v16@0:8", i_dealloc),
        m!("retain", "@16@0:8", i_retain),
        m!("release", "Vv16@0:8", i_release),
        m!("autorelease", "@16@0:8", i_autorelease),
        m!("retainCount", "Q16@0:8", i_retain_count),
        m!("class", "#16@0:8", i_class),
        m!("superclass", "#16@0:8", i_superclass),
        m!("self", "@16@0:8", i_self),
        m!("isKindOfClass:", "c24@0:8#16", i_is_kind),
        m!("isMemberOfClass:", "c24@0:8#16", i_is_member),
        m!("respondsToSelector:", "c24@0:8:16", i_responds),
        m!("conformsToProtocol:", "c24@0:8@16", i_conforms),
        m!("hash", "Q16@0:8", i_hash),
        m!("isEqual:", "c24@0:8@16", i_is_equal),
        m!("performSelector:", "@24@0:8:16", i_perform0),
        m!("performSelector:withObject:", "@32@0:8:16@24", i_perform1),
        m!("performSelector:withObject:withObject:", "@40@0:8:16@24@32", i_perform2),
        m!("zone", "^{_NSZone=}16@0:8", i_zone),
        m!("isProxy", "c16@0:8", i_no),
        m!("allowsWeakReference", "c16@0:8", i_allows_weak),
        m!("retainWeakReference", "c16@0:8", i_allows_weak),
        m!("_isDeallocating", "c16@0:8", i_is_deallocating),
        m!("copy", "@16@0:8", i_copy),
        m!("mutableCopy", "@16@0:8", i_mutable_copy),
        m!("doesNotRecognizeSelector:", "v24@0:8:16", i_not_recognized),
        m!("finalize", "v16@0:8", i_nothing),
        m!("methodForSelector:", "^?24@0:8:16", i_method_for),
        m!(".cxx_destruct", "v16@0:8", i_nothing),
    ],
});

static CLASS_METHODS: Shared<List<14>> = Shared(List {
    entsize: 24,
    count: 14,
    entries: [
        m!("alloc", "@16@0:8", c_alloc),
        m!("allocWithZone:", "@24@0:8^{_NSZone=}16", c_alloc_with_zone),
        m!("new", "@16@0:8", c_new),
        m!("init", "@16@0:8", i_self),
        m!("class", "#16@0:8", i_self),
        m!("superclass", "#16@0:8", c_superclass),
        m!("initialize", "v16@0:8", i_nothing),
        m!("instancesRespondToSelector:", "c24@0:8:16", c_instances_respond),
        m!("isSubclassOfClass:", "c24@0:8#16", c_is_subclass),
        m!("resolveInstanceMethod:", "c24@0:8:16", c_no1),
        m!("resolveClassMethod:", "c24@0:8:16", c_no1),
        m!("conformsToProtocol:", "c24@0:8@16", c_conforms),
        m!("instanceMethodForSelector:", "^?24@0:8:16", c_instance_method_for),
        m!("retainCount", "Q16@0:8", c_retain_count),
    ],
});

static CLASS_RO: Shared<Ro> = Shared(Ro {
    flags: 2,
    instance_start: 0,
    instance_size: 8,
    reserved: 0,
    ivar_layout: 0,
    name: NAME.as_ptr(),
    methods: &raw const INSTANCE_METHODS.0 as *const u8,
    protocols: 0,
    ivars: 0,
    weak_ivar_layout: 0,
    properties: 0,
});

static META_RO: Shared<Ro> = Shared(Ro {
    flags: 3,
    instance_start: 40,
    instance_size: 40,
    reserved: 0,
    ivar_layout: 0,
    name: NAME.as_ptr(),
    methods: &raw const CLASS_METHODS.0 as *const u8,
    protocols: 0,
    ivars: 0,
    weak_ivar_layout: 0,
    properties: 0,
});

/// `_OBJC_METACLASS_$_NSObject`: the root metaclass (its own class, its
/// superclass the class).
#[unsafe(export_name = "OBJC_METACLASS_$_NSObject")]
pub static META: Shared<ClassObj> = Shared(ClassObj {
    isa: &raw const META.0,
    superclass: &raw const CLASS.0,
    cache: &raw const EMPTY_CACHE.0 as *const u64,
    vtable: 0,
    data: &raw const META_RO.0,
});

/// `_OBJC_CLASS_$_NSObject`.
#[unsafe(export_name = "OBJC_CLASS_$_NSObject")]
pub static CLASS: Shared<ClassObj> = Shared(ClassObj {
    isa: &raw const META.0,
    superclass: core::ptr::null(),
    cache: &raw const EMPTY_CACHE.0 as *const u64,
    vtable: 0,
    data: &raw const CLASS_RO.0,
});

/// This library's class list: the runtime maps NSObject with the image.
#[used]
#[unsafe(link_section = "__DATA,__objc_classlist,regular,no_dead_strip")]
static CLASS_LIST: Shared<[*const ClassObj; 1]> = Shared([&raw const CLASS.0]);

fn is_class(obj: u64) -> bool {
    lock().rt.class(obj).is_some()
}

// ---- instance methods ----------------------------------------------------

extern "C" fn i_self(this: u64, _c: u64) -> u64 {
    this
}
extern "C" fn i_nothing(_this: u64, _c: u64) {}
extern "C" fn i_no(_this: u64, _c: u64) -> i8 {
    0
}
extern "C" fn i_zone(_this: u64, _c: u64) -> u64 {
    0
}
extern "C" fn i_dealloc(this: u64, _c: u64) {
    api::object_dispose(this);
}
extern "C" fn i_retain(this: u64, _c: u64) -> u64 {
    retain_obj(this)
}
extern "C" fn i_release(this: u64, _c: u64) {
    release_obj(this);
}
extern "C" fn i_autorelease(this: u64, _c: u64) -> u64 {
    autorelease_obj(this)
}
extern "C" fn i_retain_count(this: u64, _c: u64) -> u64 {
    if is_class(this) {
        return u64::MAX;
    }
    lock().refs.count(this)
}
extern "C" fn i_class(this: u64, _c: u64) -> u64 {
    Raw.u64(this)
}
extern "C" fn i_superclass(this: u64, _c: u64) -> u64 {
    let cls = Raw.u64(this);
    lock().rt.class(cls).map_or(0, |c| c.superclass)
}
extern "C" fn i_is_kind(this: u64, _c: u64, cls: u64) -> i8 {
    let mine = Raw.u64(this);
    i8::from(lock().rt.is_subclass(mine, cls))
}
extern "C" fn i_is_member(this: u64, _c: u64, cls: u64) -> i8 {
    i8::from(Raw.u64(this) == cls)
}
extern "C" fn i_responds(this: u64, _c: u64, s: u64) -> i8 {
    i8::from(api::responds(Raw.u64(this), s))
}
extern "C" fn i_conforms(this: u64, _c: u64, proto: u64) -> i8 {
    i8::from(proto != 0 && lock().rt.conforms(&Raw, Raw.u64(this), proto))
}
extern "C" fn i_hash(this: u64, _c: u64) -> u64 {
    this
}
extern "C" fn i_is_equal(this: u64, _c: u64, other: u64) -> i8 {
    i8::from(this == other)
}
extern "C" fn i_perform0(this: u64, _c: u64, s: u64) -> u64 {
    send0(this, s)
}
extern "C" fn i_perform1(this: u64, _c: u64, s: u64, a: u64) -> u64 {
    send1(this, s, a)
}
extern "C" fn i_perform2(this: u64, _c: u64, s: u64, a: u64, b: u64) -> u64 {
    // SAFETY: objc_msgSend takes the method's own signature.
    let f: extern "C" fn(u64, u64, u64, u64) -> u64 =
        unsafe { core::mem::transmute(crate::msgsend::objc_msgSend as *const ()) };
    f(this, s, a, b)
}
extern "C" fn i_allows_weak(this: u64, _c: u64) -> i8 {
    i8::from(!lock().refs.is_deallocating(this))
}
extern "C" fn i_is_deallocating(this: u64, _c: u64) -> i8 {
    i8::from(lock().refs.is_deallocating(this))
}
extern "C" fn i_copy(this: u64, _c: u64) -> u64 {
    send1(this, sel(b"copyWithZone:"), 0)
}
extern "C" fn i_mutable_copy(this: u64, _c: u64) -> u64 {
    send1(this, sel(b"mutableCopyWithZone:"), 0)
}
extern "C" fn i_not_recognized(this: u64, _c: u64, s: u64) {
    unrecognized(Raw.u64(this), s, this)
}
extern "C" fn i_method_for(this: u64, _c: u64, s: u64) -> u64 {
    api::class_getMethodImplementation(Raw.u64(this), s)
}

// ---- class methods -------------------------------------------------------

extern "C" fn c_alloc(this: u64, _c: u64) -> u64 {
    send1(this, sel(b"allocWithZone:"), 0)
}
extern "C" fn c_alloc_with_zone(this: u64, _c: u64, _zone: u64) -> u64 {
    api::class_createInstance(this, 0)
}
extern "C" fn c_new(this: u64, _c: u64) -> u64 {
    let o = send0(this, sel(b"alloc"));
    send0(o, sel(b"init"))
}
extern "C" fn c_superclass(this: u64, _c: u64) -> u64 {
    lock().rt.class(this).map_or(0, |c| c.superclass)
}
extern "C" fn c_instances_respond(this: u64, _c: u64, s: u64) -> i8 {
    i8::from(api::responds(this, s))
}
extern "C" fn c_is_subclass(this: u64, _c: u64, other: u64) -> i8 {
    i8::from(lock().rt.is_subclass(this, other))
}
extern "C" fn c_no1(_this: u64, _c: u64, _s: u64) -> i8 {
    0
}
extern "C" fn c_conforms(this: u64, _c: u64, proto: u64) -> i8 {
    i8::from(proto != 0 && lock().rt.conforms(&Raw, this, proto))
}
extern "C" fn c_instance_method_for(this: u64, _c: u64, s: u64) -> u64 {
    api::class_getMethodImplementation(this, s)
}
extern "C" fn c_retain_count(_this: u64, _c: u64) -> u64 {
    u64::MAX
}
