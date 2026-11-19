//! The `HRESULT`s of the Task Scheduler's object model (`winerror.h`; the `SCHED_*` codes are the
//! Task Scheduler's own, also in `winerror.h`).

pub type Hresult = i32;

pub const S_OK: Hresult = 0;
pub const S_FALSE: Hresult = 1;
pub const E_NOTIMPL: Hresult = 0x8000_4001_u32 as i32;
pub const E_NOINTERFACE: Hresult = 0x8000_4002_u32 as i32;
pub const E_POINTER: Hresult = 0x8000_4003_u32 as i32;
pub const E_FAIL: Hresult = 0x8000_4005_u32 as i32;
pub const E_UNEXPECTED: Hresult = 0x8000_FFFF_u32 as i32;
pub const E_ACCESSDENIED: Hresult = 0x8007_0005_u32 as i32;
pub const E_OUTOFMEMORY: Hresult = 0x8007_000E_u32 as i32;
pub const E_INVALIDARG: Hresult = 0x8007_0057_u32 as i32;
pub const CLASS_E_NOAGGREGATION: Hresult = 0x8004_0110_u32 as i32;
pub const CLASS_E_CLASSNOTAVAILABLE: Hresult = 0x8004_0111_u32 as i32;
pub const DISP_E_TYPEMISMATCH: Hresult = 0x8002_0005_u32 as i32;

/// `HRESULT_FROM_WIN32` of the Win32 errors the sheets saw.
pub const fn from_win32(e: u32) -> Hresult {
    if e == 0 { 0 } else { ((e & 0xFFFF) | 0x8007_0000) as i32 }
}

/// `ERROR_FILE_NOT_FOUND`: no such task or folder.
pub const NOT_FOUND: Hresult = from_win32(2);
/// `ERROR_PATH_NOT_FOUND`: no item of that name in a collection.
pub const PATH_NOT_FOUND: Hresult = from_win32(3);
/// `ERROR_ALREADY_EXISTS`.
pub const ALREADY_EXISTS: Hresult = from_win32(183);
/// `ERROR_INVALID_NAME`.
pub const INVALID_NAME: Hresult = from_win32(123);
/// `ERROR_DIR_NOT_EMPTY`.
pub const DIR_NOT_EMPTY: Hresult = from_win32(145);
/// `ERROR_BAD_NETPATH`: a server that is not this machine.
pub const BAD_NETPATH: Hresult = from_win32(53);
/// `ERROR_CONNECTION_INVALID`: the service object is not connected.
pub const NOT_CONNECTED: Hresult = from_win32(1251);
/// `ERROR_NONE_MAPPED`: an account name that does not exist.
pub const NONE_MAPPED: Hresult = from_win32(1332);
/// `RPC_X_NULL_REF_POINTER`: a NULL definition.
pub const NULL_REF_POINTER: Hresult = from_win32(1780);
/// `RPC_S_SERVER_UNAVAILABLE`: the scheduler process cannot be reached.
pub const SERVER_UNAVAILABLE: Hresult = from_win32(1722);

/// `SCHED_S_TASK_RUNNING`: the last result while the task runs.
pub const SCHED_S_TASK_RUNNING: Hresult = 0x0004_1301;
/// `SCHED_S_TASK_HAS_NOT_RUN`.
pub const SCHED_S_TASK_HAS_NOT_RUN: Hresult = 0x0004_1303;
/// `SCHED_S_TASK_TERMINATED`: the last result after Stop.
pub const SCHED_S_TASK_TERMINATED: Hresult = 0x0004_1306;
/// `SCHED_E_TASK_NOT_RUNNING`.
pub const SCHED_E_TASK_NOT_RUNNING: Hresult = 0x8004_130B_u32 as i32;
/// `SCHED_E_UNEXPECTEDNODE`: an element that is not allowed there.
pub const SCHED_E_UNEXPECTEDNODE: Hresult = 0x8004_1316_u32 as i32;
/// `SCHED_E_NAMESPACE`: a namespace that is not the task namespace.
pub const SCHED_E_NAMESPACE: Hresult = 0x8004_1317_u32 as i32;
/// `SCHED_E_INVALIDVALUE`: a value that is not valid.
pub const SCHED_E_INVALIDVALUE: Hresult = 0x8004_1318_u32 as i32;
/// `SCHED_E_MISSINGNODE`: a required element is missing.
pub const SCHED_E_MISSINGNODE: Hresult = 0x8004_1319_u32 as i32;
/// `SCHED_E_MALFORMEDXML`.
pub const SCHED_E_MALFORMEDXML: Hresult = 0x8004_131A_u32 as i32;
/// `SCHED_E_TASK_DISABLED`.
pub const SCHED_E_TASK_DISABLED: Hresult = 0x8004_1326_u32 as i32;
/// `SCHED_E_START_ON_DEMAND`.
pub const SCHED_E_START_ON_DEMAND: Hresult = 0x8004_1328_u32 as i32;
