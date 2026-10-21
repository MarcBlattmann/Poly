//! Thread-local and fiber-local storage (Learn's `processthreadsapi.h`
//! TLS pages and `fibersapi.h`), ADR 0079's second slice.
//!
//! TLS values live where `winternl.h` puts them: `TEB.TlsSlots[64]` and
//! the 1,024 `TlsExpansionSlots` (allocated from the process heap the first
//! time a thread stores one), `poly_kernelbase::tls`. `kernelbase` keeps
//! the list of the process's threads' TEBs (from its `DllMain`'s attach
//! and detach notifications, which our `ntdll` sends, ADR 0071) so
//! `TlsFree` can clear an index in every thread: a reused index starts at
//! zero, as Learn says.
//!
//! FLS: Poly has no fibers yet, so each thread has one set of FLS values,
//! kept here by TEB. A thread's callbacks run (for its non-null values)
//! when it detaches, and `FlsFree` runs the index's callback for every
//! thread's non-null value (to be confirmed by the spec agent: Learn says
//! the callback runs "when the fiber is deleted, when the thread exits,
//! and when an FLS index is freed").

use crate::base::*;
use crate::nt;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use poly_kernelbase::error::*;
use poly_kernelbase::tls::{self, Indices, Slot};
use poly_rtl::critsec::CriticalSection;

/// `PFLS_CALLBACK_FUNCTION`.
type FlsCallback = unsafe extern "system" fn(*mut u8);

struct Storage {
    tls: Option<Indices>,
    fls: Option<Indices>,
    callbacks: BTreeMap<u32, usize>,
    /// FLS values by thread (TEB).
    fls_values: BTreeMap<u64, Vec<usize>>,
    /// Every live thread's TEB.
    tebs: Vec<u64>,
}

struct Shared {
    lock: CriticalSection,
    state: UnsafeCell<Storage>,
}

// SAFETY: `state` is only touched with `lock` held (`with`).
unsafe impl Sync for Shared {}

static SHARED: Shared = Shared {
    lock: CriticalSection::new(),
    state: UnsafeCell::new(Storage {
        tls: None,
        fls: None,
        callbacks: BTreeMap::new(),
        fls_values: BTreeMap::new(),
        tebs: Vec::new(),
    }),
};

/// Run `f` with the storage state under its lock. `f` must not call
/// anything that takes it again.
fn with<R>(f: impl FnOnce(&mut Storage) -> R) -> R {
    let cs = &SHARED.lock as *const CriticalSection as *mut CriticalSection;
    // SAFETY: a static critical section.
    unsafe { nt::RtlEnterCriticalSection(cs) };
    // SAFETY: the lock gives exclusive access.
    let r = f(unsafe { &mut *SHARED.state.get() });
    // SAFETY: we entered it.
    unsafe { nt::RtlLeaveCriticalSection(cs) };
    r
}

/// The address of TLS `index`'s value in the thread whose TEB is `teb`;
/// `create` allocates the expansion slots if they are missing. `None` if
/// they are and `create` is false (the value is zero).
fn slot_address(teb: u64, index: u32, create: bool) -> Option<*mut usize> {
    match tls::slot(index)? {
        Slot::Teb(off) => Some((teb + off) as *mut usize),
        Slot::Expansion(i) => {
            let at = (teb + tls::TEB_TLS_EXPANSION_SLOTS) as *mut *mut usize;
            // SAFETY: a field of a live TEB.
            let mut array = unsafe { at.read() };
            if array.is_null() {
                if !create {
                    return None;
                }
                const HEAP_ZERO_MEMORY: u32 = 0x8;
                let bytes = tls::TLS_EXPANSION_SLOTS as usize * 8;
                // SAFETY: a valid heap.
                array = unsafe { nt::RtlAllocateHeap(process_heap(), HEAP_ZERO_MEMORY, bytes) } as *mut usize;
                if array.is_null() {
                    return None;
                }
                // SAFETY: as above.
                unsafe { at.write(array) };
            }
            // SAFETY: `i` < 1024, the array's length.
            Some(unsafe { array.add(i) })
        }
    }
}

/// A thread starts (`DLL_PROCESS_ATTACH` for the first, then
/// `DLL_THREAD_ATTACH`).
pub fn thread_attach() {
    let me = teb();
    with(|s| {
        if !s.tebs.contains(&me) {
            s.tebs.push(me);
        }
    });
}

/// A thread ends (`DLL_THREAD_DETACH`): its FLS callbacks, then its
/// expansion slots and its place in the list go.
pub fn thread_detach() {
    let me = teb();
    let (values, callbacks) = with(|s| {
        s.tebs.retain(|&t| t != me);
        (s.fls_values.remove(&me).unwrap_or_default(), s.callbacks.clone())
    });
    for (i, v) in values.into_iter().enumerate() {
        if let (true, Some(&cb)) = (v != 0, callbacks.get(&(i as u32))) {
            // SAFETY: the callback the program registered with FlsAlloc.
            unsafe { core::mem::transmute::<usize, FlsCallback>(cb)(v as *mut u8) };
        }
    }
    let at = (me + tls::TEB_TLS_EXPANSION_SLOTS) as *mut *mut u8;
    // SAFETY: this thread's TEB; the array came from the process heap.
    unsafe {
        let array = at.read();
        if !array.is_null() {
            at.write(core::ptr::null_mut());
            nt::RtlFreeHeap(process_heap(), 0, array);
        }
    }
}

/// `TlsAlloc`: the lowest free index, or `TLS_OUT_OF_INDEXES` with
/// `ERROR_NO_MORE_ITEMS`.
#[unsafe(no_mangle)]
pub extern "system" fn TlsAlloc() -> u32 {
    match with(|s| s.tls.get_or_insert_with(|| Indices::new(tls::TLS_MAXIMUM)).alloc()) {
        Some(i) => i,
        None => fail(ERROR_NO_MORE_ITEMS, tls::TLS_OUT_OF_INDEXES),
    }
}

/// `TlsFree`: the index's slot is cleared in every thread.
#[unsafe(no_mangle)]
pub extern "system" fn TlsFree(index: u32) -> i32 {
    with(|s| {
        if !s.tls.as_mut().is_some_and(|t| t.free(index)) {
            return fail(ERROR_INVALID_PARAMETER, FALSE);
        }
        for &t in &s.tebs {
            if let Some(p) = slot_address(t, index, false) {
                // SAFETY: a slot of a live thread's TEB (or its array).
                unsafe { p.write(0) };
            }
        }
        TRUE
    })
}

/// `TlsGetValue`: Learn: on success the last error is cleared (so a zero
/// value can be told from a failure). An index out of range is
/// `ERROR_INVALID_PARAMETER`; an in-range index is not checked for being
/// allocated (to be confirmed).
#[unsafe(no_mangle)]
pub extern "system" fn TlsGetValue(index: u32) -> *mut u8 {
    if index >= tls::TLS_MAXIMUM {
        return fail(ERROR_INVALID_PARAMETER, core::ptr::null_mut());
    }
    set_last_error(ERROR_SUCCESS);
    match slot_address(teb(), index, false) {
        // SAFETY: a slot of this thread's TEB (or its array).
        Some(p) => unsafe { p.read() as *mut u8 },
        None => core::ptr::null_mut(),
    }
}

/// `TlsSetValue`.
#[unsafe(no_mangle)]
pub extern "system" fn TlsSetValue(index: u32, value: *mut u8) -> i32 {
    if index >= tls::TLS_MAXIMUM {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    match slot_address(teb(), index, true) {
        Some(p) => {
            // SAFETY: as above.
            unsafe { p.write(value as usize) };
            TRUE
        }
        None => fail(ERROR_NOT_ENOUGH_MEMORY, FALSE),
    }
}

/// `FlsAlloc`.
#[unsafe(no_mangle)]
pub extern "system" fn FlsAlloc(callback: Option<FlsCallback>) -> u32 {
    let r = with(|s| {
        let i = s.fls.get_or_insert_with(|| Indices::new(tls::FLS_MAXIMUM_AVAILABLE)).alloc()?;
        match callback {
            Some(cb) => s.callbacks.insert(i, cb as usize),
            None => s.callbacks.remove(&i),
        };
        Some(i)
    });
    r.unwrap_or_else(|| fail(ERROR_NO_MORE_ITEMS, tls::FLS_OUT_OF_INDEXES))
}

/// `FlsFree`: every thread's non-null value goes to the callback, then
/// the index is free.
#[unsafe(no_mangle)]
pub extern "system" fn FlsFree(index: u32) -> i32 {
    let taken = with(|s| {
        if !s.fls.as_mut().is_some_and(|f| f.free(index)) {
            return None;
        }
        let cb = s.callbacks.remove(&index);
        let mut values = Vec::new();
        for v in s.fls_values.values_mut() {
            if let Some(slot) = v.get_mut(index as usize) {
                if *slot != 0 {
                    values.push(*slot);
                }
                *slot = 0;
            }
        }
        Some((cb, values))
    });
    let Some((cb, values)) = taken else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    if let Some(cb) = cb {
        for v in values {
            // SAFETY: the callback the program registered with FlsAlloc.
            unsafe { core::mem::transmute::<usize, FlsCallback>(cb)(v as *mut u8) };
        }
    }
    TRUE
}

/// `FlsGetValue`: as `TlsGetValue`, for an allocated index.
#[unsafe(no_mangle)]
pub extern "system" fn FlsGetValue(index: u32) -> *mut u8 {
    let me = teb();
    let r = with(|s| {
        if !s.fls.as_ref().is_some_and(|f| f.is_allocated(index)) {
            return None;
        }
        Some(s.fls_values.get(&me).and_then(|v| v.get(index as usize)).copied().unwrap_or(0))
    });
    match r {
        Some(v) => {
            set_last_error(ERROR_SUCCESS);
            v as *mut u8
        }
        None => fail(ERROR_INVALID_PARAMETER, core::ptr::null_mut()),
    }
}

/// `FlsSetValue`.
#[unsafe(no_mangle)]
pub extern "system" fn FlsSetValue(index: u32, value: *mut u8) -> i32 {
    let me = teb();
    let ok = with(|s| {
        if !s.fls.as_ref().is_some_and(|f| f.is_allocated(index)) {
            return false;
        }
        let v = s.fls_values.entry(me).or_default();
        if v.len() <= index as usize {
            v.resize(index as usize + 1, 0);
        }
        v[index as usize] = value as usize;
        true
    });
    if ok { TRUE } else { fail(ERROR_INVALID_PARAMETER, FALSE) }
}
