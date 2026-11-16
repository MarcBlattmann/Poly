//! Run-time type information (ADR 0199; the spec sheets `vcruntime140.dll/__RTDynamicCast`,
//! `__RTtypeid`, `__RTCastToVoid`, `__std_type_info_name`, `__std_type_info_hash`): the
//! decisions are `poly_cxxeh::rtti` and `poly_cxxeh::undname`, read through this process's
//! memory; this module throws `std::bad_cast` and `std::bad_typeid`, whose RTTI and throw
//! records it writes into its own image when the DLL is attached (`rtti::write_chain`).
//!
//! The exception objects have the layout of the public `vcruntime_exception.h`: a virtual
//! table pointer, then `__std_exception_data { const char* _What; bool _DoFree; }` (24 bytes);
//! the virtual table holds the scalar deleting destructor and `what()`, in that order.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicPtr, AtomicU64, Ordering};
use poly_cxxeh::rtti::{self, ChainClass};
use poly_cxxeh::undname;
use poly_win32_cxxrt as cxxrt;

unsafe extern "C" {
    /// The shared C++ runtime's `throw` (`poly-win32-cxxrt`'s assembly, linked into this DLL).
    fn _CxxThrowException(object: *mut u8, throw_info: *const u8) -> !;
}

/// `what()` of an exception without text (as `std::exception`'s default constructor leaves it).
static UNKNOWN: &[u8] = b"Unknown exception\0";
static BAD_CAST: &[u8] = b"Bad dynamic_cast!\0";
static BAD_TYPEID: &[u8] = b"Attempted a typeid of nullptr pointer!\0";
static NO_RTTI: &[u8] = b"Access violation - no RTTI data!\0";

/// Where the records live: in this DLL's image, so that their rvas are from its base.
#[repr(C, align(16))]
struct Arena(UnsafeCell<[u8; 2048]>);
// SAFETY: written once, at DLL_PROCESS_ATTACH (the loader lock is held, no other thread runs
// this DLL's code yet); only read afterwards.
unsafe impl Sync for Arena {}
static ARENA: Arena = Arena(UnsafeCell::new([0; 2048]));

/// `type_info`'s virtual table for our own type descriptors: its one slot is the destructor,
/// which has nothing to do.
static TYPE_INFO_VTABLE: [AtomicU64; 1] = [AtomicU64::new(0)];

/// The virtual tables and ThrowInfos made at attach: `bad_cast`, `exception` (as the base of
/// `bad_cast`), `bad_typeid`.
static VT_BAD_CAST: AtomicU64 = AtomicU64::new(0);
static VT_EXCEPTION: AtomicU64 = AtomicU64::new(0);
static VT_BAD_TYPEID: AtomicU64 = AtomicU64::new(0);
static TI_BAD_CAST: AtomicU64 = AtomicU64::new(0);
static TI_BAD_TYPEID: AtomicU64 = AtomicU64::new(0);

extern "C" fn type_info_dtor(this: *mut u8, _flags: u32) -> *mut u8 {
    this
}

/// The scalar deleting destructor: free the text it owns, and the object if asked (flag 1).
unsafe extern "C" fn exception_deleting_dtor(this: *mut u8, flags: u32) -> *mut u8 {
    // SAFETY: an exception object (24 bytes) made by us or copied by us.
    unsafe {
        super::__std_exception_destroy(this.add(8));
        if flags & 1 != 0 {
            super::free(this);
        }
    }
    this
}

/// The ThrowInfo's destructor.
unsafe extern "C" fn exception_dtor(this: *mut u8) {
    // SAFETY: as above.
    unsafe { super::__std_exception_destroy(this.add(8)) };
}

/// `what()`: the text, or the default text when there is none.
unsafe extern "C" fn exception_what(this: *const u8) -> *const u8 {
    // SAFETY: as above.
    let what = unsafe { this.add(8).cast::<*const u8>().read_unaligned() };
    if what.is_null() { UNKNOWN.as_ptr() } else { what }
}

/// The copy constructor of each class: its own virtual table, the text copied.
///
/// # Safety
/// `to` has room for an exception object; `from` is one.
unsafe fn copy_with(vtable: &AtomicU64, to: *mut u8, from: *const u8) -> *mut u8 {
    // SAFETY: the caller's objects.
    unsafe {
        to.cast::<u64>().write_unaligned(vtable.load(Ordering::Acquire));
        super::__std_exception_copy(from.add(8), to.add(8));
    }
    to
}

unsafe extern "C" fn copy_bad_cast(to: *mut u8, from: *const u8) -> *mut u8 {
    // SAFETY: called by the frame handler with the catch object and the exception object.
    unsafe { copy_with(&VT_BAD_CAST, to, from) }
}

unsafe extern "C" fn copy_exception(to: *mut u8, from: *const u8) -> *mut u8 {
    // SAFETY: as above.
    unsafe { copy_with(&VT_EXCEPTION, to, from) }
}

unsafe extern "C" fn copy_bad_typeid(to: *mut u8, from: *const u8) -> *mut u8 {
    // SAFETY: as above.
    unsafe { copy_with(&VT_BAD_TYPEID, to, from) }
}

/// Write the records of `bad_cast : exception` and `bad_typeid : exception` (from the
/// DLL's entry point, with its image base).
pub fn attach(image_base: u64) {
    let tiv = &raw const TYPE_INFO_VTABLE as u64;
    TYPE_INFO_VTABLE[0]
        .store(type_info_dtor as extern "C" fn(*mut u8, u32) -> *mut u8 as usize as u64, Ordering::Release);
    let dtor = exception_deleting_dtor as unsafe extern "C" fn(*mut u8, u32) -> *mut u8 as usize as u64;
    let what = exception_what as unsafe extern "C" fn(*const u8) -> *const u8 as usize as u64;
    let copy = |f: unsafe extern "C" fn(*mut u8, *const u8) -> *mut u8| f as usize as u64;
    let destructor = exception_dtor as unsafe extern "C" fn(*mut u8) as usize as u64;
    let virtuals = [dtor, what];
    let arena = ARENA.0.get();
    // SAFETY: the arena is ours and written only here (see `Arena`).
    let buf = unsafe { &mut *arena };
    let addr = arena as u64;
    let cast = [
        ChainClass { name: b".?AVbad_cast@std@@", virtuals: &virtuals, copy: copy(copy_bad_cast) },
        ChainClass { name: b".?AVexception@std@@", virtuals: &virtuals, copy: copy(copy_exception) },
    ];
    let Ok((chain, used)) = rtti::write_chain(buf, addr, image_base, tiv, &cast, 24, destructor) else { return };
    VT_BAD_CAST.store(chain.vtables[0], Ordering::Release);
    VT_EXCEPTION.store(chain.vtables[1], Ordering::Release);
    TI_BAD_CAST.store(chain.throw_info, Ordering::Release);
    let used = used.div_ceil(16) * 16;
    let typeid = [
        ChainClass { name: b".?AVbad_typeid@std@@", virtuals: &virtuals, copy: copy(copy_bad_typeid) },
        ChainClass { name: b".?AVexception@std@@", virtuals: &virtuals, copy: copy(copy_exception) },
    ];
    let Ok((chain, _)) =
        rtti::write_chain(&mut buf[used..], addr + used as u64, image_base, tiv, &typeid, 24, destructor)
    else {
        return;
    };
    VT_BAD_TYPEID.store(chain.vtables[0], Ordering::Release);
    TI_BAD_TYPEID.store(chain.throw_info, Ordering::Release);
}

/// Throw an exception object of the class whose virtual table and ThrowInfo are given, with a
/// literal text.
fn throw(vtable: &AtomicU64, throw_info: &AtomicU64, text: &'static [u8]) -> ! {
    let ti = throw_info.load(Ordering::Acquire);
    if ti == 0 {
        super::terminate();
    }
    let mut object = [vtable.load(Ordering::Acquire), text.as_ptr() as u64, 0];
    // SAFETY: an exception object of that class (24 bytes, the text not owned), which stays
    // in this frame until the exception is over; the ThrowInfo made at attach.
    unsafe { _CxxThrowException(object.as_mut_ptr().cast(), ti as *const u8) }
}

/// `void* __RTDynamicCast(void* inptr, LONG VfDelta, void* SrcType, void* TargetType, BOOL
/// isReference)`: `dynamic_cast`; a failed cast to a reference throws `std::bad_cast`.
#[unsafe(no_mangle)]
pub extern "C" fn __RTDynamicCast(
    inptr: *mut u8,
    vf_delta: i32,
    source: *const u8,
    target: *const u8,
    is_reference: i32,
) -> *mut u8 {
    let r = rtti::dynamic_cast(cxxrt::memory(), inptr as u64, vf_delta, source as u64, target as u64);
    match r {
        Ok(Some(p)) => p as *mut u8,
        _ if is_reference != 0 => throw(&VT_BAD_CAST, &TI_BAD_CAST, BAD_CAST),
        _ => core::ptr::null_mut(),
    }
}

/// `type_info* __RTtypeid(void* inptr)`: the type descriptor of the most derived class of a
/// polymorphic object; `std::bad_typeid` for null.
#[unsafe(no_mangle)]
pub extern "C" fn __RTtypeid(inptr: *mut u8) -> *const u8 {
    if inptr.is_null() {
        throw(&VT_BAD_TYPEID, &TI_BAD_TYPEID, BAD_TYPEID);
    }
    match rtti::dynamic_type(cxxrt::memory(), inptr as u64) {
        Ok(td) => td as *const u8,
        Err(_) => throw(&VT_BAD_TYPEID, &TI_BAD_TYPEID, NO_RTTI),
    }
}

/// `void* __RTCastToVoid(void* inptr)`: `dynamic_cast<void*>`, the complete object.
#[unsafe(no_mangle)]
pub extern "C" fn __RTCastToVoid(inptr: *mut u8) -> *mut u8 {
    if inptr.is_null() {
        return inptr;
    }
    rtti::complete_object(cxxrt::memory(), inptr as u64).map_or(core::ptr::null_mut(), |(c, _)| c as *mut u8)
}

// ----- type_info ---------------------------------------------------------------------------------

/// `size_t __std_type_info_hash(const __std_type_info_data*)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_type_info_hash(data: *const u8) -> usize {
    // SAFETY: a type descriptor's data.
    undname::type_hash(unsafe { super::decorated(data) }) as usize
}

/// `const char* __std_type_info_name(__std_type_info_data* data, __type_info_node* root)`:
/// the readable name, made once per type and kept in the descriptor's first field
/// (`_UndecoratedName`); the allocation is linked into the module's list at `root` (whose
/// first 8 bytes this DLL keeps as the head of a list of its blocks), which
/// `__std_type_info_destroy_list` frees.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_type_info_name(data: *mut u8, root: *mut u8) -> *const u8 {
    // SAFETY: `_UndecoratedName` is the first, pointer-aligned field of the data.
    let slot = unsafe { &*data.cast::<AtomicPtr<u8>>() };
    let cached = slot.load(Ordering::Acquire);
    if !cached.is_null() {
        return cached;
    }
    // SAFETY: a type descriptor's data.
    let decorated = unsafe { super::decorated(data) };
    let name = undname::type_name(decorated);
    let text: &[u8] = match &name {
        Some(n) => n.as_bytes(),
        None => decorated.strip_prefix(b".").unwrap_or(decorated),
    };
    // A block: the link to the next block (16 bytes, keeping the text aligned), the text.
    // SAFETY: an allocation of the size written below.
    let block = unsafe { super::malloc(16 + text.len() + 1) };
    if block.is_null() {
        return decorated.as_ptr();
    }
    // SAFETY: the block is `16 + len + 1` bytes.
    let name_ptr = unsafe {
        core::ptr::copy_nonoverlapping(text.as_ptr(), block.add(16), text.len());
        block.add(16 + text.len()).write(0);
        block.add(16)
    };
    match slot.compare_exchange(core::ptr::null_mut(), name_ptr, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => {
            if !root.is_null() {
                // SAFETY: the root node is at least pointer-sized and aligned (16 bytes).
                let head = unsafe { &*root.cast::<AtomicPtr<u8>>() };
                let mut old = head.load(Ordering::Acquire);
                loop {
                    // SAFETY: our block's link field.
                    unsafe { block.cast::<*mut u8>().write(old) };
                    match head.compare_exchange_weak(old, block, Ordering::AcqRel, Ordering::Acquire) {
                        Ok(_) => break,
                        Err(now) => old = now,
                    }
                }
            }
            name_ptr
        }
        Err(other) => {
            // Another thread named it first.
            // SAFETY: our own block, not published.
            unsafe { super::free(block) };
            other
        }
    }
}

/// `void __std_type_info_destroy_list(__type_info_node* root)`: free the names made for the
/// module whose list this is.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_type_info_destroy_list(root: *mut u8) {
    if root.is_null() {
        return;
    }
    // SAFETY: as in `__std_type_info_name`.
    let head = unsafe { &*root.cast::<AtomicPtr<u8>>() };
    let mut block = head.swap(core::ptr::null_mut(), Ordering::AcqRel);
    let mut n = 0u32;
    while !block.is_null() && n < 1 << 20 {
        // SAFETY: a block of ours: its link field, then the block freed.
        unsafe {
            let next = block.cast::<*mut u8>().read();
            super::free(block);
            block = next;
        }
        n += 1;
    }
}
