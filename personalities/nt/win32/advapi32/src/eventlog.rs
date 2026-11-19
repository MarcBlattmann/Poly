//! The event log (ADR 0227): `RegisterEventSourceA/W`, `ReportEventA/W`,
//! `DeregisterEventSource`, `OpenEventLogA/W`, `ReadEventLogW` and
//! `CloseEventLog`. The formats and decisions are `poly_kernelbase::eventlog`'s
//! (host-tested); this file keeps the process's handle table and reads and
//! appends the log files, `%ProgramData%\Poly\EventLog\<log>.evtlog`.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_kernelbase::eventlog::{self as el, Entry, Handles};

type Handle = isize;

#[cfg_attr(target_arch = "x86", link(name = "kernelbase", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernelbase", kind = "raw-dylib"))]
unsafe extern "system" {
    fn SetLastError(error: u32);
    fn GetLastError() -> u32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *const u8,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn ReadFile(h: Handle, buf: *mut u8, len: u32, read: *mut u32, overlapped: *mut u8) -> i32;
    fn WriteFile(h: Handle, buf: *const u8, len: u32, written: *mut u32, overlapped: *mut u8) -> i32;
    fn SetFilePointerEx(h: Handle, distance: i64, new: *mut i64, method: u32) -> i32;
    fn SetEndOfFile(h: Handle) -> i32;
    fn CloseHandle(h: Handle) -> i32;
    fn CreateDirectoryW(name: *const u16, security: *const u8) -> i32;
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, size: u32) -> u32;
    fn GetComputerNameW(buf: *mut u16, size: *mut u32) -> i32;
    fn GetSystemTimeAsFileTime(out: *mut u64);
    fn Sleep(ms: u32);
}

/// `GENERIC_READ`, `GENERIC_WRITE`, `FILE_SHARE_READ | FILE_SHARE_WRITE`,
/// `OPEN_EXISTING`, `OPEN_ALWAYS`, `FILE_ATTRIBUTE_NORMAL`, `FILE_BEGIN`
/// (`winnt.h`, `fileapi.h`); `ERROR_SHARING_VIOLATION`, `ERROR_PATH_NOT_FOUND`
/// (`winerror.h`).
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const SHARE_READ_WRITE: u32 = 0x3;
const OPEN_EXISTING: u32 = 3;
const OPEN_ALWAYS: u32 = 4;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const FILE_BEGIN: u32 = 0;
const ERROR_SHARING_VIOLATION: u32 = 32;
const ERROR_PATH_NOT_FOUND: u32 = 3;
const INVALID_HANDLE_VALUE: Handle = -1;
/// 100 ns intervals from 1601 to 1970.
const FILETIME_1970: u64 = 116_444_736_000_000_000;

/// The process's event log handles, behind a spin lock.
struct Table {
    locked: AtomicBool,
    handles: core::cell::UnsafeCell<Handles>,
}

// SAFETY: access to `handles` is serialised by `locked`.
unsafe impl Sync for Table {}

static TABLE: Table = Table { locked: AtomicBool::new(false), handles: core::cell::UnsafeCell::new(Handles::new()) };

fn with_table<R>(f: impl FnOnce(&mut Handles) -> R) -> R {
    while TABLE.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: the lock is held.
    let r = f(unsafe { &mut *TABLE.handles.get() });
    TABLE.locked.store(false, Ordering::Release);
    r
}

fn fail<T>(error: u32, value: T) -> T {
    // SAFETY: plain call.
    unsafe { SetLastError(error) };
    value
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn nul(mut s: Vec<u16>) -> Vec<u16> {
    s.push(0);
    s
}

/// A NUL-terminated wide string from the caller (`None` for a null pointer).
///
/// # Safety
/// `p` is null or a NUL-terminated UTF-16 string.
unsafe fn wide(p: *const u16) -> Option<Vec<u16>> {
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    // SAFETY: the caller's contract: the scan stops at the NUL.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        Some(core::slice::from_raw_parts(p, n).to_vec())
    }
}

/// A NUL-terminated ANSI string from the caller, as UTF-16.
///
/// # Safety
/// `p` is null or a NUL-terminated string.
unsafe fn ansi(p: *const u8) -> Option<Vec<u16>> {
    if p.is_null() {
        return None;
    }
    // SAFETY: the caller's contract.
    let bytes = unsafe { core::slice::from_raw_parts(p, poly_kernelbase::text::strlen(p)) };
    Some(poly_kernelbase::text::from_ansi(bytes))
}

fn computer_name() -> Vec<u16> {
    let mut buf = [0u16; 64];
    let mut n = buf.len() as u32;
    // SAFETY: the buffer and its size.
    if unsafe { GetComputerNameW(buf.as_mut_ptr(), &mut n) } == 0 {
        return w("POLY");
    }
    buf[..n as usize].to_vec()
}

/// `%ProgramData%\Poly\EventLog` (made on the first write).
fn log_dir() -> Vec<u16> {
    let name = nul(w("ProgramData"));
    let mut buf = [0u16; 260];
    // SAFETY: the buffer and its size.
    let n = unsafe { GetEnvironmentVariableW(name.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    let mut dir = if n == 0 || n >= buf.len() { w("C:\\ProgramData") } else { buf[..n].to_vec() };
    dir.extend(w("\\Poly\\EventLog"));
    dir
}

fn log_path(log: &str) -> Vec<u16> {
    let mut p = log_dir();
    p.push(u16::from(b'\\'));
    p.extend(w(log));
    p.extend(w(el::LOG_EXTENSION));
    nul(p)
}

/// Make the log directory and its parents (those that exist stay).
fn make_log_dir() {
    let dir = log_dir();
    for (i, &c) in dir.iter().enumerate() {
        if c == u16::from(b'\\') && i > 2 {
            let part = nul(dir[..i].to_vec());
            // SAFETY: a NUL-terminated path.
            unsafe { CreateDirectoryW(part.as_ptr(), core::ptr::null()) };
        }
    }
    let all = nul(dir);
    // SAFETY: a NUL-terminated path.
    unsafe { CreateDirectoryW(all.as_ptr(), core::ptr::null()) };
}

fn read_all(h: Handle) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chunk = alloc::vec![0u8; 16 * 1024];
    loop {
        let mut got = 0u32;
        // SAFETY: the handle and a buffer of the length given.
        let ok = unsafe { ReadFile(h, chunk.as_mut_ptr(), chunk.len() as u32, &mut got, core::ptr::null_mut()) };
        if ok == 0 || got == 0 {
            return out;
        }
        out.extend_from_slice(&chunk[..got as usize]);
    }
}

/// The bytes of log `log`'s file (none when it does not exist yet).
fn read_log(log: &str) -> Vec<u8> {
    let path = log_path(log);
    // SAFETY: a NUL-terminated path.
    let h = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ,
            SHARE_READ_WRITE,
            core::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            0,
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let bytes = read_all(h);
    // SAFETY: the handle just opened.
    unsafe { CloseHandle(h) };
    bytes
}

/// Append `e` to log `log`, numbering it after the newest record; the file is
/// open without sharing meanwhile (another writer waits). The Win32 error on
/// failure.
fn append(log: &str, e: &el::Event) -> Result<(), u32> {
    let path = log_path(log);
    let mut tries = 0;
    let h = loop {
        // SAFETY: a NUL-terminated path.
        let h = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                core::ptr::null(),
                OPEN_ALWAYS,
                FILE_ATTRIBUTE_NORMAL,
                0,
            )
        };
        if h != INVALID_HANDLE_VALUE {
            break h;
        }
        // SAFETY: plain call.
        let error = unsafe { GetLastError() };
        tries += 1;
        match error {
            ERROR_PATH_NOT_FOUND if tries < 3 => make_log_dir(),
            // SAFETY: plain call.
            ERROR_SHARING_VIOLATION if tries < 2000 => unsafe { Sleep(1) },
            _ => return Err(error),
        }
    };
    let bytes = read_all(h);
    let end = el::end_of_records(&bytes);
    let record = el::encode(e, el::next_number(&bytes));
    let mut written = 0u32;
    // SAFETY: the handle just opened; the record is a live buffer.
    let ok = unsafe {
        SetFilePointerEx(h, end as i64, core::ptr::null_mut(), FILE_BEGIN) != 0
            && WriteFile(h, record.as_ptr(), record.len() as u32, &mut written, core::ptr::null_mut()) != 0
            && SetEndOfFile(h) != 0
    };
    // SAFETY: plain call.
    let error = if ok { 0 } else { unsafe { GetLastError() } };
    // SAFETY: the handle just opened.
    unsafe { CloseHandle(h) };
    if ok { Ok(()) } else { Err(error) }
}

fn now_1970() -> u32 {
    let mut ft = 0u64;
    // SAFETY: the out variable.
    unsafe { GetSystemTimeAsFileTime(&mut ft) };
    (ft.saturating_sub(FILETIME_1970) / 10_000_000) as u32
}

// ----- sources ------------------------------------------------------------------------------

fn register(server: Option<Vec<u16>>, source: Option<Vec<u16>>) -> Handle {
    if !el::is_local_server(server.as_deref(), &computer_name()) {
        return fail(el::RPC_S_SERVER_UNAVAILABLE, 0);
    }
    let Some(name) = source.filter(|s| !s.is_empty()) else { return fail(el::ERROR_INVALID_PARAMETER, 0) };
    with_table(|t| t.add(Entry::Source { name })) as Handle
}

/// `RegisterEventSourceW(lpUNCServerName, lpSourceName)`
/// (`RegisterEventSourceW.yaml`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegisterEventSourceW(server: *const u16, source: *const u16) -> Handle {
    // SAFETY: the caller's strings or null.
    unsafe { register(wide(server), wide(source)) }
}

/// `RegisterEventSourceA` (`RegisterEventSourceA.yaml`): any source name gives
/// a new handle, an unregistered one too.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegisterEventSourceA(server: *const u8, source: *const u8) -> Handle {
    // SAFETY: the caller's strings or null.
    unsafe { register(ansi(server), ansi(source)) }
}

/// `DeregisterEventSource(hEventLog)`: `ERROR_INVALID_HANDLE` for a handle
/// that is not open; the last error is left alone on success.
#[unsafe(no_mangle)]
pub extern "system" fn DeregisterEventSource(h: Handle) -> i32 {
    match with_table(|t| t.remove(h as usize)) {
        Some(_) => 1,
        None => fail(el::ERROR_INVALID_HANDLE, 0),
    }
}

/// `ReportEvent`'s work, the strings already UTF-16.
///
/// # Safety
/// `sid` is null or a SID; `data` holds `size` bytes (or is null).
unsafe fn report(
    h: Handle,
    kind: u16,
    category: u16,
    id: u32,
    sid: *const u8,
    strings: Vec<Vec<u16>>,
    size: u32,
    data: *const u8,
) -> i32 {
    let source = with_table(|t| match t.get_mut(h as usize) {
        Some(Entry::Source { name }) => Some(name.clone()),
        _ => None,
    });
    let Some(source) = source else { return fail(el::ERROR_INVALID_HANDLE, 0) };
    if !el::valid_type(kind) {
        return fail(el::ERROR_BAD_EVENT_TYPE, 0);
    }
    // SAFETY: the caller's contract.
    let (sid, data) = unsafe {
        let sid = if sid.is_null() {
            Vec::new()
        } else {
            // A SID is 8 bytes and 4 per sub-authority (`winnt.h`).
            core::slice::from_raw_parts(sid, 8 + 4 * usize::from(*sid.add(1))).to_vec()
        };
        let data = if data.is_null() || size == 0 {
            Vec::new()
        } else {
            core::slice::from_raw_parts(data, size as usize).to_vec()
        };
        (sid, data)
    };
    let e = el::Event {
        source,
        computer: computer_name(),
        event_id: id,
        kind,
        category,
        strings,
        sid,
        data,
        time: now_1970(),
    };
    match append(el::APPLICATION, &e) {
        Ok(()) => 1,
        Err(error) => fail(error, 0),
    }
}

/// `ReportEventW` (`ReportEventW.yaml`).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn ReportEventW(
    h: Handle,
    kind: u16,
    category: u16,
    id: u32,
    sid: *const u8,
    count: u16,
    size: u32,
    strings: *const *const u16,
    data: *const u8,
) -> i32 {
    let mut list = Vec::new();
    for i in 0..usize::from(if strings.is_null() { 0 } else { count }) {
        // SAFETY: the caller's array of `count` strings.
        list.push(unsafe { wide(*strings.add(i)) }.unwrap_or_default());
    }
    // SAFETY: the caller's SID and data.
    unsafe { report(h, kind, category, id, sid, list, size, data) }
}

/// `ReportEventA` (`ReportEventA.yaml`): the strings are stored as UTF-16
/// (code page 1252: the byte E9 is U+00E9); a type outside the six event
/// types is refused with 23 and writes nothing.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn ReportEventA(
    h: Handle,
    kind: u16,
    category: u16,
    id: u32,
    sid: *const u8,
    count: u16,
    size: u32,
    strings: *const *const u8,
    data: *const u8,
) -> i32 {
    let mut list = Vec::new();
    for i in 0..usize::from(if strings.is_null() { 0 } else { count }) {
        // SAFETY: the caller's array of `count` strings.
        list.push(unsafe { ansi(*strings.add(i)) }.unwrap_or_default());
    }
    // SAFETY: the caller's SID and data.
    unsafe { report(h, kind, category, id, sid, list, size, data) }
}

// ----- reading ------------------------------------------------------------------------------

fn open_log(server: Option<Vec<u16>>, name: Option<Vec<u16>>) -> Handle {
    if !el::is_local_server(server.as_deref(), &computer_name()) {
        return fail(el::RPC_S_SERVER_UNAVAILABLE, 0);
    }
    let Some(name) = name else { return fail(el::ERROR_INVALID_PARAMETER, 0) };
    let log = el::log_of(&name);
    with_table(|t| t.add(Entry::Reader { log, cursor: el::Cursor::default() })) as Handle
}

/// `OpenEventLogW(lpUNCServerName, lpSourceName)`: a reading handle on the
/// System log or, for any other name, the Application log
/// (`EventLogSourceRegistration.yaml` `read_back`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn OpenEventLogW(server: *const u16, name: *const u16) -> Handle {
    // SAFETY: the caller's strings or null.
    unsafe { open_log(wide(server), wide(name)) }
}

/// `OpenEventLogA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn OpenEventLogA(server: *const u8, name: *const u8) -> Handle {
    // SAFETY: the caller's strings or null.
    unsafe { open_log(ansi(server), ansi(name)) }
}

/// `CloseEventLog(hEventLog)`.
#[unsafe(no_mangle)]
pub extern "system" fn CloseEventLog(h: Handle) -> i32 {
    DeregisterEventSource(h)
}

/// `ReadEventLogW(hEventLog, dwReadFlags, dwRecordOffset, lpBuffer,
/// nNumberOfBytesToRead, pnBytesRead, pnMinNumberOfBytesNeeded)`
/// (`poly_kernelbase::eventlog::read`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ReadEventLogW(
    h: Handle,
    flags: u32,
    offset: u32,
    buf: *mut u8,
    len: u32,
    read: *mut u32,
    needed: *mut u32,
) -> i32 {
    if read.is_null() || needed.is_null() || (buf.is_null() && len != 0) {
        return fail(el::ERROR_INVALID_PARAMETER, 0);
    }
    let reader = with_table(|t| match t.get_mut(h as usize) {
        Some(Entry::Reader { log, cursor }) => Some((*log, *cursor)),
        _ => None,
    });
    let Some((log, mut cursor)) = reader else { return fail(el::ERROR_INVALID_HANDLE, 0) };
    let file = read_log(log);
    let r = el::read(&file, &mut cursor, flags, offset, len as usize);
    with_table(|t| {
        if let Some(Entry::Reader { cursor: c, .. }) = t.get_mut(h as usize) {
            *c = cursor;
        }
    });
    // SAFETY: the caller's out-parameters (checked non-null) and buffer of `len` bytes.
    unsafe {
        match r {
            Ok(bytes) => {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, bytes.len());
                read.write(bytes.len() as u32);
                needed.write(0);
                1
            }
            Err((error, need)) => {
                read.write(0);
                needed.write(need);
                fail(error, 0)
            }
        }
    }
}
