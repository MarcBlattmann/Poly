//! The taskbar list (`CLSID_TaskbarList`: `ITaskbarList` .. `ITaskbarList4`) and the jump list
//! (`CLSID_DestinationList`: `ICustomDestinationList`) Chrome makes for every window and at
//! start-up (ADR 0223). Poly's panel has no progress bars on its buttons and no jump lists
//! yet: every call succeeds as on Windows and changes nothing that can be seen. The jump
//! list keeps the one state its callers can observe, whether a list is begun. Sheets:
//! `specs/windows/shell32.dll/ITaskbarList3.SetProgressValue.yaml`,
//! `specs/windows/shell32.dll/ICustomDestinationList.BeginList.yaml`.

use crate::com::*;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering};

/// `IID_ITaskbarList`, `IID_ITaskbarList2`, `IID_ITaskbarList3`, `IID_ITaskbarList4` (`shobjidl_core.h`).
const IID_ITASKBAR_LIST: Guid = guid(0x56FD_F342, 0xFD6D, 0x11D0, [0x95, 0x8A, 0x00, 0x60, 0x97, 0xC9, 0xA0, 0x90]);
const IID_ITASKBAR_LIST2: Guid = guid(0x602D_4995, 0xB13A, 0x429B, [0xA6, 0x6E, 0x19, 0x35, 0xE4, 0x4F, 0x43, 0x17]);
const IID_ITASKBAR_LIST3: Guid = guid(0xEA1A_FB91, 0x9E28, 0x4B86, [0x90, 0xE9, 0x9E, 0x9F, 0x8A, 0x5E, 0xEF, 0xAF]);
const IID_ITASKBAR_LIST4: Guid = guid(0xC43D_C798, 0x95D1, 0x4BEA, [0x90, 0x30, 0xBB, 0x99, 0xE2, 0x98, 0x3A, 0x1A]);
/// `IID_ICustomDestinationList` (`shobjidl_core.h`).
const IID_ICUSTOM_DESTINATION_LIST: Guid =
    guid(0x6332_DEBF, 0x87B5, 0x4670, [0x90, 0xC0, 0x5E, 0x57, 0xB4, 0x08, 0xA4, 0x9E]);
/// `IID_IObjectArray` (`objectarray.h`).
const IID_IOBJECT_ARRAY: Guid = guid(0x92CA_9DCD, 0x5622, 0x4BBA, [0xA8, 0x05, 0x5E, 0x9F, 0x54, 0x1B, 0xD8, 0xC9]);
/// `KDC_FREQUENT`, `KDC_RECENT` (`shobjidl_core.h`).
const KDC_FREQUENT: u32 = 1;
const KDC_RECENT: u32 = 2;
/// The jump list's slots Poly reports (`BeginList`'s `pcMinSlots`).
const MIN_SLOTS: u32 = 10;

// ----- the taskbar list -----

#[repr(C)]
struct TaskbarList {
    head: Head,
}

static TASKBAR_IIDS: [Guid; 4] = [IID_ITASKBAR_LIST, IID_ITASKBAR_LIST2, IID_ITASKBAR_LIST3, IID_ITASKBAR_LIST4];

unsafe extern "system" fn ok_0(_: *mut c_void) -> Hresult {
    S_OK
}
unsafe extern "system" fn ok_1(_: *mut c_void, _: usize) -> Hresult {
    S_OK
}
unsafe extern "system" fn ok_2(_: *mut c_void, _: usize, _: usize) -> Hresult {
    S_OK
}
unsafe extern "system" fn ok_3(_: *mut c_void, _: usize, _: usize, _: usize) -> Hresult {
    S_OK
}
/// `SetProgressValue(hwnd, ULONGLONG completed, ULONGLONG total)`: two 64-bit numbers (on x86
/// each takes two argument slots).
unsafe extern "system" fn set_progress_value(_: *mut c_void, _: usize, _: u64, _: u64) -> Hresult {
    S_OK
}

/// `ITaskbarList` after `IUnknown`: `HrInit` 3, `AddTab` 4, `DeleteTab` 5, `ActivateTab` 6,
/// `SetActiveAlt` 7; `ITaskbarList2`: `MarkFullscreenWindow` 8; `ITaskbarList3`:
/// `SetProgressValue` 9, `SetProgressState` 10, `RegisterTab` 11, `UnregisterTab` 12,
/// `SetTabOrder` 13, `SetTabActive` 14, `ThumbBarAddButtons` 15, `ThumbBarUpdateButtons` 16,
/// `ThumbBarSetImageList` 17, `SetOverlayIcon` 18, `SetThumbnailTooltip` 19,
/// `SetThumbnailClip` 20; `ITaskbarList4`: `SetTabProperties` 21.
static TASKBAR_VTBL: Vtbl<22> = Vtbl([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    ok_0 as *const c_void,
    ok_1 as *const c_void,
    ok_1 as *const c_void,
    ok_1 as *const c_void,
    ok_1 as *const c_void,
    ok_2 as *const c_void,
    set_progress_value as *const c_void,
    ok_2 as *const c_void,
    ok_2 as *const c_void,
    ok_1 as *const c_void,
    ok_2 as *const c_void,
    ok_3 as *const c_void,
    ok_3 as *const c_void,
    ok_3 as *const c_void,
    ok_2 as *const c_void,
    ok_3 as *const c_void,
    ok_2 as *const c_void,
    ok_2 as *const c_void,
    ok_2 as *const c_void,
]);

/// The class factory's `CreateInstance` for `CLSID_TaskbarList`: a new object each time.
pub unsafe fn create_taskbar_list(iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    let object = into_raw(TaskbarList { head: Head::new(&TASKBAR_VTBL, &TASKBAR_IIDS, drop_box::<TaskbarList>) });
    // SAFETY: a fresh object; the caller's out parameter.
    unsafe { hand_out(object.cast(), iid, out) }
}

// ----- an empty object array -----

#[repr(C)]
struct EmptyArray {
    head: Head,
}

static ARRAY_IIDS: [Guid; 1] = [IID_IOBJECT_ARRAY];

/// `IObjectArray` after `IUnknown`: `GetCount` 3, `GetAt` 4.
static ARRAY_VTBL: Vtbl<5> = Vtbl([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    array_count as *const c_void,
    array_at as *const c_void,
]);

unsafe extern "system" fn array_count(_: *mut c_void, count: *mut u32) -> Hresult {
    if count.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *count = 0 };
    S_OK
}

unsafe extern "system" fn array_at(_: *mut c_void, _: u32, _: *const Guid, out: *mut *mut c_void) -> Hresult {
    if !out.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *out = core::ptr::null_mut() };
    }
    E_INVALIDARG
}

/// An empty `IObjectArray` (or another of its interfaces) for the caller.
unsafe fn empty_array(iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    let object = into_raw(EmptyArray { head: Head::new(&ARRAY_VTBL, &ARRAY_IIDS, drop_box::<EmptyArray>) });
    // SAFETY: a fresh object; the caller's out parameter.
    unsafe { hand_out(object.cast(), iid, out) }
}

// ----- the jump list -----

#[repr(C)]
struct DestinationList {
    head: Head,
    /// A list is begun (`BeginList` ran, no `CommitList` or `AbortList` since).
    begun: AtomicBool,
}

static LIST_IIDS: [Guid; 1] = [IID_ICUSTOM_DESTINATION_LIST];

/// `ICustomDestinationList` after `IUnknown`: `SetAppID` 3, `BeginList` 4, `AppendCategory` 5,
/// `AppendKnownCategory` 6, `AddUserTasks` 7, `CommitList` 8, `GetRemovedDestinations` 9,
/// `DeleteList` 10, `AbortList` 11.
static LIST_VTBL: Vtbl<12> = Vtbl([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    ok_1 as *const c_void,
    begin_list as *const c_void,
    ok_2 as *const c_void,
    append_known_category as *const c_void,
    ok_1 as *const c_void,
    commit_list as *const c_void,
    get_removed as *const c_void,
    ok_1 as *const c_void,
    abort_list as *const c_void,
]);

/// The class factory's `CreateInstance` for `CLSID_DestinationList`.
pub unsafe fn create_destination_list(iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    let object = into_raw(DestinationList {
        head: Head::new(&LIST_VTBL, &LIST_IIDS, drop_box::<DestinationList>),
        begun: AtomicBool::new(false),
    });
    // SAFETY: a fresh object; the caller's out parameter.
    unsafe { hand_out(object.cast(), iid, out) }
}

unsafe fn begun<'a>(p: *mut c_void) -> &'a AtomicBool {
    // SAFETY: our object.
    unsafe { &(*p.cast::<DestinationList>()).begun }
}

unsafe extern "system" fn begin_list(
    p: *mut c_void,
    slots: *mut u32,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> Hresult {
    if slots.is_null() || out.is_null() {
        return E_POINTER;
    }
    // SAFETY: our object; the caller's pointers.
    unsafe {
        *out = core::ptr::null_mut();
        if begun(p).load(Ordering::Acquire) {
            return E_UNEXPECTED;
        }
        let hr = empty_array(iid, out);
        if hr != S_OK {
            return hr;
        }
        *slots = MIN_SLOTS;
        begun(p).store(true, Ordering::Release);
    }
    S_OK
}

unsafe extern "system" fn append_known_category(p: *mut c_void, category: u32) -> Hresult {
    // SAFETY: our object.
    if !unsafe { begun(p) }.load(Ordering::Acquire) {
        return E_UNEXPECTED;
    }
    if category == KDC_FREQUENT || category == KDC_RECENT { S_OK } else { E_FAIL }
}

/// `CommitList` and `AbortList` end a begun list; with none: `E_UNEXPECTED`.
unsafe extern "system" fn commit_list(p: *mut c_void) -> Hresult {
    // SAFETY: our object.
    if unsafe { begun(p) }.swap(false, Ordering::AcqRel) { S_OK } else { E_UNEXPECTED }
}

unsafe extern "system" fn abort_list(p: *mut c_void) -> Hresult {
    // SAFETY: our object.
    unsafe { commit_list(p) }
}

unsafe extern "system" fn get_removed(_: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { empty_array(iid, out) }
}
