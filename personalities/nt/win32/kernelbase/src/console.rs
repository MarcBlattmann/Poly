//! The console functions (Learn: "Console Functions" and each function's
//! page; ADR 0080) over Poly's console host: console handles are file
//! objects on `\Device\PolyConsole`, and everything but `ReadFile` /
//! `WriteFile` is an `NtDeviceIoControlFile` with Poly's codes
//! (`poly_console::ioctl`). The console itself — modes, line editing, the
//! screen buffer — runs in the kernel's NT personality.
//!
//! - The process's console connection and standard handles come from its
//!   parameters ([`crate::process::init_std_handles`]); at
//!   `DLL_PROCESS_ATTACH` this module registers [`ctrl_routine`] with the
//!   console, which starts a thread there for each control event (Learn:
//!   "HandlerRoutine": "the system creates a new thread in the process").
//! - A console function on a handle that is not a console handle fails
//!   with `ERROR_INVALID_HANDLE` (Learn: "WriteConsole": "WriteConsole
//!   fails if it is used with a standard handle that is redirected to a
//!   file").
//! - `ReadConsoleW` ended by Ctrl+C succeeds with nothing read and
//!   `ERROR_OPERATION_ABORTED` as the last error (Poly's choice, to be
//!   confirmed by the spec agent).
//! - `AllocConsole` makes a console on Poly's system console terminal and
//!   `AttachConsole` attaches to one; both point the three standard handles
//!   at it (Learn: "AllocConsole": "initializes standard input, standard
//!   output, and standard error handles for the new console").
//!   `FreeConsole` leaves the standard handles alone (to be confirmed).
//! - `GetConsoleWindow` is NULL: a Poly console has no window (Learn: "the
//!   return value is NULL if there is no such associated console window").

use crate::base::*;
use crate::nt::{self, Handle, IoStatusBlock};
use alloc::vec::Vec;
use poly_console::consts::{ATTACH_PARENT_PROCESS, INPUT_RECORD_SIZE, STATUS_CONTROL_C_EXIT};
use poly_console::ioctl;
use poly_kernelbase::error::*;
use poly_ntobj::NtStatus;

const STATUS_INVALID_DEVICE_REQUEST: NtStatus = NtStatus(0xC000_0010);
const STATUS_BUFFER_OVERFLOW: NtStatus = NtStatus(0x8000_0005);
const STATUS_CANCELLED: NtStatus = NtStatus(0xC000_0120);

/// One control request: the output bytes, or the status.
fn control(h: Handle, code: u32, input: &[u8], out: &mut [u8]) -> Result<usize, NtStatus> {
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid buffers of the lengths given.
    let mut st = unsafe {
        nt::NtDeviceIoControlFile(
            h,
            0,
            core::ptr::null(),
            core::ptr::null(),
            &mut iosb,
            code,
            input.as_ptr(),
            input.len() as u32,
            out.as_mut_ptr(),
            out.len() as u32,
        )
    };
    if st == nt::STATUS_PENDING {
        // An asynchronous handle: wait for the request.
        // SAFETY: a handle value.
        unsafe { nt::NtWaitForSingleObject(h, 0, core::ptr::null()) };
        st = NtStatus(iosb.status as u32);
    }
    if st.is_success() || st == STATUS_BUFFER_OVERFLOW { Ok(iosb.information as usize) } else { Err(st) }
}

/// Fail a console function: a handle that is not a console's is
/// `ERROR_INVALID_HANDLE`.
fn fail_console<T>(st: NtStatus, value: T) -> T {
    if st == STATUS_INVALID_DEVICE_REQUEST { fail(ERROR_INVALID_HANDLE, value) } else { fail_status(st, value) }
}

fn get_u32(h: Handle, code: u32, input: &[u8]) -> Result<u32, NtStatus> {
    let mut out = [0u8; 4];
    control(h, code, input, &mut out)?;
    Ok(u32::from_le_bytes(out))
}

/// Run a request that answers nothing: TRUE, or FALSE and the error.
fn simple(h: Handle, code: u32, input: &[u8]) -> i32 {
    match control(h, code, input, &mut []) {
        Ok(_) => TRUE,
        Err(st) => fail_console(st, FALSE),
    }
}

fn console() -> Handle {
    with_state(|s| s.console)
}

// ----- attaching ---------------------------------------------------------------------

/// Open `name` (relative to `root` when non-zero) as a synchronous console
/// handle.
fn open(root: Handle, name: &str) -> Result<Handle, NtStatus> {
    use poly_kernelbase::file::*;
    let units = poly_kernelbase::wide(name);
    let string = nt::UnicodeString::new(&units);
    let mut oa = nt::ObjectAttributes::new(Some(&string), nt::OBJ_CASE_INSENSITIVE);
    oa.root = root;
    let mut h: Handle = 0;
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers.
    let st = unsafe {
        nt::NtCreateFile(
            &mut h,
            GENERIC_READ | GENERIC_WRITE | SYNCHRONIZE,
            &oa,
            &mut iosb,
            core::ptr::null(),
            0,
            3,
            FILE_OPEN,
            FILE_SYNCHRONOUS_IO_NONALERT,
            core::ptr::null(),
            0,
        )
    };
    if st.is_success() { Ok(h) } else { Err(st) }
}

/// Become attached through connection `conn`: the control routine, the
/// standard handles.
fn attach(conn: Handle) -> Result<(), NtStatus> {
    register(conn);
    let input = open(conn, ioctl::INPUT)?;
    let output = open(conn, ioctl::OUTPUT)?;
    let error = open(conn, ioctl::OUTPUT)?;
    with_state(|s| {
        s.console = conn;
        s.std = [input, output, error];
    });
    Ok(())
}

/// Tell the console where control events go.
fn register(conn: Handle) {
    let routine = ctrl_routine as *const () as usize as u64;
    let _ = control(conn, ioctl::SET_CTRL_ROUTINE, &routine.to_le_bytes(), &mut []);
}

/// At `DLL_PROCESS_ATTACH`, with the connection from the parameters.
pub fn init(conn: Handle) {
    if conn == 0 {
        return;
    }
    register(conn);
    if let Ok(id) = get_u32(conn, ioctl::GET_ID, &[]) {
        with_state(|s| s.parent_console = id);
    }
}

/// `AllocConsole`: `ERROR_ACCESS_DENIED` when the process has a console
/// (Learn: "A process can be associated with only one console").
#[unsafe(no_mangle)]
pub extern "system" fn AllocConsole() -> i32 {
    if console() != 0 {
        return fail(ERROR_ACCESS_DENIED, FALSE);
    }
    let r = open(0, &alloc::format!("{}\\{}", ioctl::DEVICE, ioctl::NEW)).and_then(attach);
    match r {
        Ok(()) => TRUE,
        Err(st) => fail_status(st, FALSE),
    }
}

/// `AttachConsole`: Learn: already attached is `ERROR_ACCESS_DENIED`, a
/// process without a console `ERROR_INVALID_HANDLE`, a process that does
/// not exist `ERROR_INVALID_PARAMETER` (Poly cannot tell the last two apart
/// for a process id: `ERROR_INVALID_PARAMETER`).
#[unsafe(no_mangle)]
pub extern "system" fn AttachConsole(pid: u32) -> i32 {
    if console() != 0 {
        return fail(ERROR_ACCESS_DENIED, FALSE);
    }
    let (name, missing) = if pid == ATTACH_PARENT_PROCESS {
        let parent = with_state(|s| s.parent_console);
        if parent == 0 {
            return fail(ERROR_INVALID_HANDLE, FALSE);
        }
        (alloc::format!("{}\\{parent}", ioctl::DEVICE), ERROR_INVALID_HANDLE)
    } else {
        (alloc::format!("{}\\{}\\{pid}", ioctl::DEVICE, ioctl::PROCESS), ERROR_INVALID_PARAMETER)
    };
    let conn = match open(0, &name) {
        Ok(h) => h,
        Err(_) => return fail(missing, FALSE),
    };
    match attach(conn) {
        Ok(()) => TRUE,
        Err(st) => fail_status(st, FALSE),
    }
}

/// `FreeConsole`: detach (the console closes when no process is left on
/// it). Without a console: `ERROR_INVALID_PARAMETER` (Poly's choice, to be
/// confirmed by the spec agent).
#[unsafe(no_mangle)]
pub extern "system" fn FreeConsole() -> i32 {
    let conn = with_state(|s| core::mem::replace(&mut s.console, 0));
    if conn == 0 {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    close(conn);
    TRUE
}

/// `GetConsoleWindow`: NULL, a Poly console has no window.
#[unsafe(no_mangle)]
pub extern "system" fn GetConsoleWindow() -> Handle {
    0
}

/// `CreateFileW` of `CONIN$` / `CONOUT$` / `CON` (Learn: "CreateFile":
/// "Consoles"): a handle to the input buffer or the screen buffer; `CON`
/// is the input with `GENERIC_READ`, the output otherwise. `None`: not a
/// console name.
pub fn create_file_console(name: &[u16], access: u32) -> Option<Handle> {
    let is = |s: &str| {
        name.len() == s.len()
            && name.iter().zip(s.bytes()).all(|(&c, b)| c < 0x80 && (c as u8).eq_ignore_ascii_case(&b))
    };
    let input = if is("CONIN$") {
        true
    } else if is("CONOUT$") {
        false
    } else if is("CON") {
        access & poly_kernelbase::file::GENERIC_WRITE == 0
    } else {
        return None;
    };
    let conn = console();
    if conn == 0 {
        return Some(fail(ERROR_INVALID_HANDLE, INVALID_HANDLE_VALUE));
    }
    Some(match open(conn, if input { ioctl::INPUT } else { ioctl::OUTPUT }) {
        Ok(h) => {
            set_last_error(ERROR_SUCCESS);
            h
        }
        Err(st) => fail_status(st, INVALID_HANDLE_VALUE),
    })
}

// ----- modes --------------------------------------------------------------------------

/// `GetConsoleMode`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetConsoleMode(h: Handle, mode: *mut u32) -> i32 {
    match get_u32(h, ioctl::GET_MODE, &[]) {
        Ok(m) => {
            if mode.is_null() {
                return fail(ERROR_INVALID_PARAMETER, FALSE);
            }
            // SAFETY: the caller's out-pointer.
            unsafe { mode.write(m) };
            TRUE
        }
        Err(st) => fail_console(st, FALSE),
    }
}

/// `SetConsoleMode`.
#[unsafe(no_mangle)]
pub extern "system" fn SetConsoleMode(h: Handle, mode: u32) -> i32 {
    simple(h, ioctl::SET_MODE, &mode.to_le_bytes())
}

// ----- reading -------------------------------------------------------------------------

/// A text read: the units or bytes read, or FALSE.
///
/// # Safety
/// `buf` holds `bytes` bytes; `read` is null or writable.
unsafe fn read_text(h: Handle, code: u32, buf: *mut u8, bytes: usize, unit: usize, read: *mut u32) -> i32 {
    if buf.is_null() && bytes > 0 {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's buffer.
    let out = unsafe { core::slice::from_raw_parts_mut(buf, bytes) };
    let (n, ok) = match control(h, code, &[], out) {
        Ok(n) => (n / unit, TRUE),
        Err(STATUS_CANCELLED) => {
            set_last_error(ERROR_OPERATION_ABORTED);
            (0, TRUE)
        }
        Err(st) => (0, fail_console(st, FALSE)),
    };
    if !read.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { read.write(n as u32) };
    }
    ok
}

/// `ReadConsoleW`: `pInputControl` is ignored (Poly has no
/// `dwCtrlWakeupMask` yet).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ReadConsoleW(
    h: Handle,
    buf: *mut u16,
    chars: u32,
    read: *mut u32,
    _control: *const u8,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { read_text(h, ioctl::READ_W, buf as *mut u8, chars as usize * 2, 2, read) }
}

/// `ReadConsoleA`: bytes in the input code page.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ReadConsoleA(
    h: Handle,
    buf: *mut u8,
    chars: u32,
    read: *mut u32,
    _control: *const u8,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { read_text(h, ioctl::READ_A, buf, chars as usize, 1, read) }
}

/// `ReadConsoleInputW` (`wait`) / `PeekConsoleInputW`.
///
/// # Safety
/// `buf` holds `len` records; `read` is writable.
unsafe fn input_records(h: Handle, buf: *mut u8, len: u32, read: *mut u32, remove: bool) -> i32 {
    if read.is_null() || (buf.is_null() && len > 0) {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let flags = if remove { ioctl::READ_INPUT_REMOVE | ioctl::READ_INPUT_WAIT } else { 0 };
    // SAFETY: the caller's buffer.
    let out = unsafe { core::slice::from_raw_parts_mut(buf, len as usize * INPUT_RECORD_SIZE) };
    match control(h, ioctl::READ_INPUT, &flags.to_le_bytes(), out) {
        Ok(n) => {
            // SAFETY: the caller's out-pointer.
            unsafe { read.write((n / INPUT_RECORD_SIZE) as u32) };
            TRUE
        }
        Err(st) => {
            // SAFETY: as above.
            unsafe { read.write(0) };
            fail_console(st, FALSE)
        }
    }
}

/// `ReadConsoleInputW`: waits for at least one record.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ReadConsoleInputW(h: Handle, buf: *mut u8, len: u32, read: *mut u32) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { input_records(h, buf, len, read, true) }
}

/// `PeekConsoleInputW`: returns at once, records stay.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PeekConsoleInputW(h: Handle, buf: *mut u8, len: u32, read: *mut u32) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { input_records(h, buf, len, read, false) }
}

/// `WriteConsoleInputW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WriteConsoleInputW(h: Handle, buf: *const u8, len: u32, written: *mut u32) -> i32 {
    if buf.is_null() && len > 0 {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's records.
    let input = unsafe { core::slice::from_raw_parts(buf, len as usize * INPUT_RECORD_SIZE) };
    match get_u32(h, ioctl::WRITE_INPUT, input) {
        Ok(n) => {
            if !written.is_null() {
                // SAFETY: the caller's out-pointer.
                unsafe { written.write(n) };
            }
            TRUE
        }
        Err(st) => fail_console(st, FALSE),
    }
}

/// `GetNumberOfConsoleInputEvents`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetNumberOfConsoleInputEvents(h: Handle, n: *mut u32) -> i32 {
    match get_u32(h, ioctl::INPUT_COUNT, &[]) {
        Ok(v) => {
            if n.is_null() {
                return fail(ERROR_INVALID_PARAMETER, FALSE);
            }
            // SAFETY: the caller's out-pointer.
            unsafe { n.write(v) };
            TRUE
        }
        Err(st) => fail_console(st, FALSE),
    }
}

/// `FlushConsoleInputBuffer`.
#[unsafe(no_mangle)]
pub extern "system" fn FlushConsoleInputBuffer(h: Handle) -> i32 {
    simple(h, ioctl::FLUSH_INPUT, &[])
}

// ----- writing -------------------------------------------------------------------------

/// The most one request carries; longer writes go in pieces.
const PIECE: usize = 32 * 1024;

/// A write of `data` in pieces of whole units; the units written.
fn write_pieces(h: Handle, code: u32, data: &[u8], unit: usize) -> Result<usize, NtStatus> {
    let mut done = 0;
    for piece in data.chunks(PIECE - PIECE % unit) {
        done += get_u32(h, code, piece)? as usize;
    }
    Ok(done)
}

/// `WriteConsoleW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WriteConsoleW(
    h: Handle,
    buf: *const u16,
    chars: u32,
    written: *mut u32,
    _r: *const u8,
) -> i32 {
    if buf.is_null() && chars > 0 {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's text.
    let data = unsafe { core::slice::from_raw_parts(buf as *const u8, chars as usize * 2) };
    let r =
        if chars == 0 { get_u32(h, ioctl::WRITE_W, &[]).map(|_| 0) } else { write_pieces(h, ioctl::WRITE_W, data, 2) };
    finish_write(r, written)
}

/// `WriteConsoleA`: bytes in the output code page.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WriteConsoleA(
    h: Handle,
    buf: *const u8,
    chars: u32,
    written: *mut u32,
    _r: *const u8,
) -> i32 {
    if buf.is_null() && chars > 0 {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's text.
    let data = unsafe { core::slice::from_raw_parts(buf, chars as usize) };
    let r =
        if chars == 0 { get_u32(h, ioctl::WRITE_A, &[]).map(|_| 0) } else { write_pieces(h, ioctl::WRITE_A, data, 1) };
    finish_write(r, written)
}

fn finish_write(r: Result<usize, NtStatus>, written: *mut u32) -> i32 {
    let (n, ok) = match r {
        Ok(n) => (n, TRUE),
        Err(st) => (0, fail_console(st, FALSE)),
    };
    if !written.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { written.write(n as u32) };
    }
    ok
}

// ----- the screen buffer ---------------------------------------------------------------

fn info_ex(h: Handle) -> Result<[u8; 96], NtStatus> {
    let mut out = [0u8; 96];
    control(h, ioctl::GET_INFO, &[], &mut out)?;
    Ok(out)
}

/// `GetConsoleScreenBufferInfo` (`CONSOLE_SCREEN_BUFFER_INFO`, 22 bytes).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetConsoleScreenBufferInfo(h: Handle, info: *mut u8) -> i32 {
    match info_ex(h) {
        Ok(b) => {
            if info.is_null() {
                return fail(ERROR_INVALID_PARAMETER, FALSE);
            }
            // SAFETY: the caller's structure.
            unsafe { core::ptr::copy_nonoverlapping(b[4..26].as_ptr(), info, 22) };
            TRUE
        }
        Err(st) => fail_console(st, FALSE),
    }
}

/// `GetConsoleScreenBufferInfoEx`: `cbSize` must be
/// `sizeof(CONSOLE_SCREEN_BUFFER_INFOEX)` (Learn), else
/// `ERROR_INVALID_PARAMETER`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetConsoleScreenBufferInfoEx(h: Handle, info: *mut u8) -> i32 {
    // SAFETY: the caller's structure starts with cbSize.
    if info.is_null() || unsafe { (info as *const u32).read_unaligned() } != 96 {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    match info_ex(h) {
        Ok(b) => {
            // SAFETY: the caller's 96-byte structure.
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), info, 96) };
            TRUE
        }
        Err(st) => fail_console(st, FALSE),
    }
}

/// `SetConsoleTextAttribute`.
#[unsafe(no_mangle)]
pub extern "system" fn SetConsoleTextAttribute(h: Handle, attr: u16) -> i32 {
    simple(h, ioctl::SET_ATTRIBUTE, &u32::from(attr).to_le_bytes())
}

/// `SetConsoleCursorPosition` (`COORD` by value: X in the low 16 bits).
#[unsafe(no_mangle)]
pub extern "system" fn SetConsoleCursorPosition(h: Handle, at: u32) -> i32 {
    simple(h, ioctl::SET_CURSOR, &at.to_le_bytes())
}

fn fill(h: Handle, kind: u32, value: u32, count: u32, at: u32, written: *mut u32) -> i32 {
    let f = ioctl::Fill { kind, value, count, x: at as u16 as i16, y: (at >> 16) as u16 as i16 };
    let r = get_u32(h, ioctl::FILL, &f.to_bytes());
    let (n, ok) = match r {
        Ok(n) => (n, TRUE),
        Err(st) => (0, fail_console(st, FALSE)),
    };
    if !written.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { written.write(n) };
    }
    ok
}

/// `FillConsoleOutputCharacterW`.
#[unsafe(no_mangle)]
pub extern "system" fn FillConsoleOutputCharacterW(h: Handle, ch: u16, count: u32, at: u32, written: *mut u32) -> i32 {
    fill(h, ioctl::FILL_CHARACTER, u32::from(ch), count, at, written)
}

/// `FillConsoleOutputCharacterA`: the character in the output code page.
#[unsafe(no_mangle)]
pub extern "system" fn FillConsoleOutputCharacterA(h: Handle, ch: u8, count: u32, at: u32, written: *mut u32) -> i32 {
    let cp = get_u32(h, ioctl::GET_CP, &1u32.to_le_bytes()).unwrap_or(poly_console::consts::CP_437);
    let w = poly_console::codepage::decode(cp, &[ch]);
    fill(h, ioctl::FILL_CHARACTER, u32::from(w.first().copied().unwrap_or(0x3F)), count, at, written)
}

/// `FillConsoleOutputAttribute`.
#[unsafe(no_mangle)]
pub extern "system" fn FillConsoleOutputAttribute(h: Handle, attr: u16, count: u32, at: u32, written: *mut u32) -> i32 {
    fill(h, ioctl::FILL_ATTRIBUTE, u32::from(attr), count, at, written)
}

// ----- code pages and the title --------------------------------------------------------

fn get_cp(output: bool) -> u32 {
    let conn = console();
    if conn == 0 {
        return fail(ERROR_INVALID_HANDLE, 0);
    }
    get_u32(conn, ioctl::GET_CP, &u32::from(output).to_le_bytes()).unwrap_or_else(|st| fail_status(st, 0))
}

fn set_cp(output: bool, cp: u32) -> i32 {
    let conn = console();
    if conn == 0 {
        return fail(ERROR_INVALID_HANDLE, FALSE);
    }
    let mut input = [0u8; 8];
    input[..4].copy_from_slice(&u32::from(output).to_le_bytes());
    input[4..].copy_from_slice(&cp.to_le_bytes());
    simple(conn, ioctl::SET_CP, &input)
}

/// `GetConsoleCP`: 0 without a console.
#[unsafe(no_mangle)]
pub extern "system" fn GetConsoleCP() -> u32 {
    get_cp(false)
}

/// `GetConsoleOutputCP`.
#[unsafe(no_mangle)]
pub extern "system" fn GetConsoleOutputCP() -> u32 {
    get_cp(true)
}

/// `SetConsoleCP`: an unsupported code page is `ERROR_INVALID_PARAMETER`.
#[unsafe(no_mangle)]
pub extern "system" fn SetConsoleCP(cp: u32) -> i32 {
    set_cp(false, cp)
}

/// `SetConsoleOutputCP`.
#[unsafe(no_mangle)]
pub extern "system" fn SetConsoleOutputCP(cp: u32) -> i32 {
    set_cp(true, cp)
}

/// `SetConsoleTitleW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetConsoleTitleW(title: *const u16) -> i32 {
    // SAFETY: the caller's string.
    let Some(t) = (unsafe { wide_arg(title) }) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    let conn = console();
    if conn == 0 {
        return fail(ERROR_INVALID_HANDLE, FALSE);
    }
    let bytes: Vec<u8> = t.iter().flat_map(|u| u.to_le_bytes()).collect();
    simple(conn, ioctl::SET_TITLE, &bytes)
}

/// `GetConsoleTitleW`: the title's length in characters; a buffer too
/// small gets what fits, NUL-terminated (Poly's reading of Learn, to be
/// confirmed by the spec agent).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetConsoleTitleW(buf: *mut u16, size: u32) -> u32 {
    let conn = console();
    if conn == 0 {
        return fail(ERROR_INVALID_HANDLE, 0);
    }
    let mut out = alloc::vec![0u8; 64 * 1024];
    let n = match control(conn, ioctl::GET_TITLE, &[], &mut out) {
        Ok(n) => n / 2,
        Err(st) => return fail_status(st, 0),
    };
    let title: Vec<u16> = out[..n * 2].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    if buf.is_null() || size == 0 {
        return fail(ERROR_INSUFFICIENT_BUFFER, 0);
    }
    let copy = title.len().min(size as usize - 1);
    // SAFETY: the caller's buffer of `size` units.
    unsafe { put_wide(buf, &title[..copy]) };
    set_last_error(ERROR_SUCCESS);
    title.len() as u32
}

// ----- control events ------------------------------------------------------------------

/// `SetConsoleCtrlHandler` (`poly_console::ctrl`).
#[unsafe(no_mangle)]
pub extern "system" fn SetConsoleCtrlHandler(routine: usize, add: i32) -> i32 {
    match with_state(|s| s.ctrl.set(routine as u64, add != 0)) {
        Ok(()) => TRUE,
        Err(e) => fail(e, FALSE),
    }
}

/// `GenerateConsoleCtrlEvent`: every process on the console gets `event`
/// (process groups are not kept yet: `group` is ignored).
#[unsafe(no_mangle)]
pub extern "system" fn GenerateConsoleCtrlEvent(event: u32, _group: u32) -> i32 {
    let conn = console();
    if conn == 0 {
        return fail(ERROR_INVALID_HANDLE, FALSE);
    }
    simple(conn, ioctl::GENERATE_CTRL, &event.to_le_bytes())
}

/// A `PHANDLER_ROUTINE`.
type HandlerRoutine = unsafe extern "system" fn(u32) -> i32;

/// Where the console host starts a thread for a control event: the
/// handlers, last registered first, until one returns TRUE; none did, the
/// default handler ends the process (Learn: "HandlerRoutine").
pub extern "system" fn ctrl_routine(event: usize) -> u32 {
    let event = event as u32;
    let Some(plan) = with_state(|s| s.ctrl.plan(event)) else { return 0 };
    for routine in plan {
        // SAFETY: the program registered it as a PHANDLER_ROUTINE.
        let f: HandlerRoutine = unsafe { core::mem::transmute::<usize, HandlerRoutine>(routine as usize) };
        // SAFETY: calling the program's handler with its documented argument.
        if unsafe { f(event) } != 0 {
            return 0;
        }
    }
    // SAFETY: an ntdll export that does not return.
    unsafe { nt::RtlExitUserProcess(STATUS_CONTROL_C_EXIT) }
}
