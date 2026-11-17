//! The shell's file operation object (`CLSID_FileOperation`, `IFileOperation`), as Chrome's
//! download code renames a finished download with it (ADR 0223). Operations are queued by
//! `MoveItem`, `RenameItem`, `CopyItem` and `DeleteItem` and done by `PerformOperations`; a
//! move or rename in one volume is `MoveFileExW`'s rename, which keeps the bytes, the
//! attributes and the alternate data streams. Sheet: `specs/windows/shell32.dll/IFileOperation.MoveItem.yaml`.
//! Poly shows no progress or confirmation windows: every operation runs as with `FOF_NO_UI`.

use crate::com::*;
use crate::item::path_of;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use poly_kernelbase::shellitem as si;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    fn CopyFileW(from: *const u16, to: *const u16, fail_if_exists: i32) -> i32;
    fn DeleteFileW(name: *const u16) -> i32;
    fn RemoveDirectoryW(name: *const u16) -> i32;
    fn SetFileAttributesW(name: *const u16, attributes: u32) -> i32;
    fn FindFirstFileW(pattern: *const u16, data: *mut u8) -> usize;
    fn FindNextFileW(find: usize, data: *mut u8) -> i32;
    fn FindClose(find: usize) -> i32;
    fn GetLastError() -> u32;
}

/// `IID_IFileOperation` (`shobjidl_core.h`).
pub const IID_IFILE_OPERATION: Guid = guid(0x947A_AB5F, 0x0A5C, 0x4C13, [0xB4, 0xD6, 0x4B, 0xF7, 0x83, 0x6F, 0xC9, 0xF8]);
/// `FOF_RENAMEONCOLLISION` (`shellapi.h`).
const FOF_RENAMEONCOLLISION: u32 = 0x8;
const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
const MOVEFILE_COPY_ALLOWED: u32 = 0x2;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const INVALID_HANDLE_VALUE: usize = usize::MAX;
/// `WIN32_FIND_DATAW` (`minwinbase.h`): 592 bytes; the attributes at 0, the name at 44.
const FIND_DATA_SIZE: usize = 592;
const FIND_NAME_AT: usize = 44;

/// One queued operation, with the paths as they were when it was queued.
enum Op {
    /// Move `from` into `folder` as `name` (`None`: its own name).
    Move { from: Vec<u16>, folder: Vec<u16>, name: Option<Vec<u16>> },
    /// Give `from` a new name in its folder.
    Rename { from: Vec<u16>, name: Vec<u16> },
    Copy { from: Vec<u16>, folder: Vec<u16>, name: Option<Vec<u16>> },
    Delete { from: Vec<u16> },
}

#[derive(Default)]
struct State {
    flags: u32,
    ops: Vec<Op>,
    performed: bool,
    aborted: bool,
}

#[repr(C)]
struct FileOperation {
    head: Head,
    /// Used from the thread of the object's apartment only (the class is apartment-threaded).
    state: UnsafeCell<State>,
}

static IIDS: [Guid; 1] = [IID_IFILE_OPERATION];

/// `IFileOperation` after `IUnknown`: `Advise` 3 .. `GetAnyOperationsAborted` 22.
static VTBL: Vtbl<23> = Vtbl([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    advise as *const c_void,
    ok_1 as *const c_void, // Unadvise(cookie)
    set_operation_flags as *const c_void,
    set_progress_message as *const c_void,
    ok_1 as *const c_void, // SetProgressDialog(dialog)
    ok_1 as *const c_void, // SetProperties(changes)
    ok_1 as *const c_void, // SetOwnerWindow(hwnd)
    ok_1 as *const c_void, // ApplyPropertiesToItem(item)
    ok_1 as *const c_void, // ApplyPropertiesToItems(items)
    rename_item as *const c_void,
    not_implemented_2 as *const c_void, // RenameItems(items, name)
    move_item as *const c_void,
    not_implemented_2 as *const c_void, // MoveItems(items, folder)
    copy_item as *const c_void,
    not_implemented_2 as *const c_void, // CopyItems(items, folder)
    delete_item as *const c_void,
    not_implemented_1 as *const c_void, // DeleteItems(items)
    new_item as *const c_void,
    perform_operations as *const c_void,
    get_any_operations_aborted as *const c_void,
]);

/// The class factory's `CreateInstance` for `CLSID_FileOperation`.
pub unsafe fn create(iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    let object = into_raw(FileOperation {
        head: Head::new(&VTBL, &IIDS, drop_box::<FileOperation>),
        state: UnsafeCell::new(State::default()),
    });
    // SAFETY: a fresh object; the caller's out parameter.
    unsafe { hand_out(object.cast(), iid, out) }
}

unsafe fn state<'a>(p: *mut c_void) -> &'a mut State {
    // SAFETY: our object, used by one thread at a time (its apartment's).
    unsafe { &mut *(*p.cast::<FileOperation>()).state.get() }
}

unsafe extern "system" fn ok_1(_: *mut c_void, _: usize) -> Hresult {
    S_OK
}
unsafe extern "system" fn not_implemented_1(_: *mut c_void, _: usize) -> Hresult {
    E_NOTIMPL
}
unsafe extern "system" fn not_implemented_2(_: *mut c_void, _: usize, _: usize) -> Hresult {
    E_NOTIMPL
}

unsafe extern "system" fn advise(_: *mut c_void, sink: *mut c_void, cookie: *mut u32) -> Hresult {
    if sink.is_null() || cookie.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out parameter. Poly calls no progress sink: the operations report
    // nothing while they run.
    unsafe { *cookie = 1 };
    S_OK
}

unsafe extern "system" fn set_operation_flags(p: *mut c_void, flags: u32) -> Hresult {
    // SAFETY: our object.
    unsafe { state(p).flags = flags };
    S_OK
}

/// "Not implemented" on Windows too (the sheet's `misc`).
unsafe extern "system" fn set_progress_message(_: *mut c_void, _: *const u16) -> Hresult {
    E_NOTIMPL
}

unsafe extern "system" fn new_item(
    _: *mut c_void,
    _folder: *mut c_void,
    _attributes: u32,
    _name: *const u16,
    _template: *const u16,
    _sink: *mut c_void,
) -> Hresult {
    E_NOTIMPL
}

/// A new name of the caller (`None` for NULL).
unsafe fn name_of(name: *const u16) -> Option<Vec<u16>> {
    // SAFETY: a NUL-terminated string of the caller.
    if name.is_null() { None } else { Some(unsafe { wstr(name) }.to_vec()) }
}

unsafe extern "system" fn move_item(
    p: *mut c_void,
    item: *mut c_void,
    folder: *mut c_void,
    name: *const u16,
    _sink: *mut c_void,
) -> Hresult {
    // SAFETY: our object; the caller's items and name.
    unsafe {
        let (Some(from), Some(folder)) = (path_of(item), path_of(folder)) else { return E_INVALIDARG };
        state(p).ops.push(Op::Move { from, folder, name: name_of(name) });
    }
    S_OK
}

unsafe extern "system" fn copy_item(
    p: *mut c_void,
    item: *mut c_void,
    folder: *mut c_void,
    name: *const u16,
    _sink: *mut c_void,
) -> Hresult {
    // SAFETY: our object; the caller's items and name.
    unsafe {
        let (Some(from), Some(folder)) = (path_of(item), path_of(folder)) else { return E_INVALIDARG };
        state(p).ops.push(Op::Copy { from, folder, name: name_of(name) });
    }
    S_OK
}

unsafe extern "system" fn rename_item(p: *mut c_void, item: *mut c_void, name: *const u16, _sink: *mut c_void) -> Hresult {
    // SAFETY: our object; the caller's item and name.
    unsafe {
        let Some(from) = path_of(item) else { return E_INVALIDARG };
        if name.is_null() {
            return E_INVALIDARG;
        }
        state(p).ops.push(Op::Rename { from, name: wstr(name).to_vec() });
    }
    S_OK
}

unsafe extern "system" fn delete_item(p: *mut c_void, item: *mut c_void, _sink: *mut c_void) -> Hresult {
    // SAFETY: our object; the caller's item.
    unsafe {
        let Some(from) = path_of(item) else { return E_INVALIDARG };
        state(p).ops.push(Op::Delete { from });
    }
    S_OK
}

unsafe extern "system" fn get_any_operations_aborted(p: *mut c_void, out: *mut i32) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: our object; the caller's out parameter.
    unsafe { *out = i32::from(state(p).aborted) };
    S_OK
}

/// The target of a move or copy of `from` into `folder` as `name`: with `FOF_RENAMEONCOLLISION`
/// an existing other object's name is kept and the new one gets `<name> - Copy...`.
fn target(from: &[u16], folder: &[u16], name: Option<&[u16]>, flags: u32) -> Vec<u16> {
    let name = name.unwrap_or_else(|| si::file_name(from));
    let to = si::join(folder, name);
    if flags & FOF_RENAMEONCOLLISION == 0 || si::same_path(&to, from) || crate::item::exists(&to).is_none() {
        return to;
    }
    let mut n = 1;
    loop {
        let candidate = si::join(folder, &si::collision_name(name, n));
        if crate::item::exists(&candidate).is_none() {
            return candidate;
        }
        n += 1;
    }
}

/// What one operation came to.
enum Outcome {
    Done,
    /// Skipped (its source is gone, or the name cannot be used): `GetAnyOperationsAborted`.
    Aborted,
    Failed(u32),
}

fn move_to(from: &[u16], to: &[u16]) -> Outcome {
    if si::same_path(from, to) && from == to {
        return Outcome::Done;
    }
    let (f, t) = (nul(from), nul(to));
    // SAFETY: NUL-terminated names.
    if unsafe { MoveFileExW(f.as_ptr(), t.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_COPY_ALLOWED) } != 0 {
        Outcome::Done
    } else {
        // SAFETY: a plain call.
        Outcome::Failed(unsafe { GetLastError() })
    }
}

/// Delete a file, or a folder with everything in it.
fn delete_tree(path: &[u16]) -> Outcome {
    let Some(folder) = crate::item::exists(path) else { return Outcome::Aborted };
    let name = nul(path);
    if folder {
        let pattern = nul(&si::join(path, &[u16::from(b'*')]));
        let mut data = [0u8; FIND_DATA_SIZE];
        // SAFETY: a NUL-terminated pattern and a WIN32_FIND_DATAW-sized buffer.
        let find = unsafe { FindFirstFileW(pattern.as_ptr(), data.as_mut_ptr()) };
        if find != INVALID_HANDLE_VALUE {
            loop {
                let units: Vec<u16> = data[FIND_NAME_AT..FIND_NAME_AT + 520]
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .take_while(|&c| c != 0)
                    .collect();
                let dots = units.iter().all(|&c| c == u16::from(b'.'));
                if !dots && let Outcome::Failed(e) = delete_tree(&si::join(path, &units)) {
                    // SAFETY: the search handle.
                    unsafe { FindClose(find) };
                    return Outcome::Failed(e);
                }
                // SAFETY: the search handle and the buffer.
                if unsafe { FindNextFileW(find, data.as_mut_ptr()) } == 0 {
                    break;
                }
            }
            // SAFETY: the search handle.
            unsafe { FindClose(find) };
        }
        // SAFETY: a NUL-terminated name.
        if unsafe { RemoveDirectoryW(name.as_ptr()) } == 0 {
            // SAFETY: a plain call.
            return Outcome::Failed(unsafe { GetLastError() });
        }
        return Outcome::Done;
    }
    // SAFETY: NUL-terminated names. A read-only file goes too, as the shell deletes it.
    unsafe {
        if DeleteFileW(name.as_ptr()) == 0 {
            SetFileAttributesW(name.as_ptr(), FILE_ATTRIBUTE_NORMAL);
            if DeleteFileW(name.as_ptr()) == 0 {
                return Outcome::Failed(GetLastError());
            }
        }
    }
    Outcome::Done
}

fn run(op: &Op, flags: u32) -> Outcome {
    match op {
        Op::Move { from, folder, name } => {
            if crate::item::exists(from).is_none() {
                return Outcome::Aborted;
            }
            if name.as_deref().is_some_and(|n| !si::new_name_is_plain(n)) {
                return Outcome::Aborted;
            }
            move_to(from, &target(from, folder, name.as_deref(), flags))
        }
        Op::Rename { from, name } => {
            if crate::item::exists(from).is_none() || !si::new_name_is_plain(name) {
                return Outcome::Aborted;
            }
            let Some(folder) = si::parent(from) else { return Outcome::Aborted };
            move_to(from, &target(from, &folder, Some(name), flags))
        }
        Op::Copy { from, folder, name } => {
            match crate::item::exists(from) {
                None => return Outcome::Aborted,
                // Copying folders is not built yet.
                Some(true) => return Outcome::Aborted,
                Some(false) => {}
            }
            if name.as_deref().is_some_and(|n| !si::new_name_is_plain(n)) {
                return Outcome::Aborted;
            }
            let to = target(from, folder, name.as_deref(), flags);
            if si::same_path(from, &to) {
                return Outcome::Aborted;
            }
            let (f, t) = (nul(from), nul(&to));
            // SAFETY: NUL-terminated names.
            if unsafe { CopyFileW(f.as_ptr(), t.as_ptr(), 0) } != 0 {
                Outcome::Done
            } else {
                // SAFETY: a plain call.
                Outcome::Failed(unsafe { GetLastError() })
            }
        }
        Op::Delete { from } => delete_tree(from),
    }
}

/// `PerformOperations`: the queued operations in order. Nothing queued, or a second call:
/// `E_UNEXPECTED`. An operation whose source is gone is skipped and counts as aborted; one
/// that fails ends the run with its error as an `HRESULT`.
unsafe extern "system" fn perform_operations(p: *mut c_void) -> Hresult {
    // SAFETY: our object.
    let st = unsafe { state(p) };
    if st.performed || st.ops.is_empty() {
        return E_UNEXPECTED;
    }
    st.performed = true;
    for op in core::mem::take(&mut st.ops) {
        match run(&op, st.flags) {
            Outcome::Done => {}
            Outcome::Aborted => st.aborted = true,
            Outcome::Failed(e) => {
                st.aborted = true;
                return poly_com::hresult::from_win32(e);
            }
        }
    }
    S_OK
}
