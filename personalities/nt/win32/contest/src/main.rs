//! `contest.exe` (ADR 0080): the program the kernel's console ring-3 test
//! `nt_console_program_on_a_terminal` runs on a pseudo-terminal, with its
//! standard output redirected to a file. It imports only from
//! `kernel32.dll` and checks the console functions against their Learn
//! pages: the standard handles and `GetFileType`, modes, `WriteConsoleW`
//! and `WriteFile` in two code pages, the screen buffer (attributes, the
//! cursor, fills, `GetConsoleScreenBufferInfo(Ex)`), VT sequences, the
//! title, cooked reads from typed input (`ReadConsoleW`, `ReadFile` with a
//! backspace), raw input records, a Ctrl+C handler, `CONOUT$`, and
//! `FreeConsole` / `AttachConsole` / `AllocConsole`.
//!
//! It tells the test when to type with markers on its console:
//! `READY-INPUT` (type the lines) and `READY-CTRLC` (press Ctrl+C). It
//! exits with [`PASS`]; a failed check prints `contest: step N failed
//! (last error E)` on its standard error and exits with `0x80 + N`.

#![no_std]
#![no_main]
#![allow(clippy::missing_safety_doc)]

use core::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use poly_win32_rt as _;

type Handle = isize;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ExitProcess(code: u32) -> !;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn GetStdHandle(which: u32) -> Handle;
    fn GetFileType(h: Handle) -> u32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *const u8,
        disp: u32,
        flags: u32,
        t: Handle,
    ) -> Handle;
    fn CloseHandle(h: Handle) -> i32;
    fn WriteFile(h: Handle, buf: *const u8, len: u32, written: *mut u32, ov: *mut u8) -> i32;
    fn ReadFile(h: Handle, buf: *mut u8, len: u32, read: *mut u32, ov: *mut u8) -> i32;
    fn CreateEventW(sa: *const u8, manual: i32, initial: i32, name: *const u16) -> Handle;
    fn SetEvent(h: Handle) -> i32;
    fn WaitForSingleObject(h: Handle, ms: u32) -> u32;

    fn AllocConsole() -> i32;
    fn FreeConsole() -> i32;
    fn AttachConsole(pid: u32) -> i32;
    fn GetConsoleMode(h: Handle, mode: *mut u32) -> i32;
    fn SetConsoleMode(h: Handle, mode: u32) -> i32;
    fn ReadConsoleW(h: Handle, buf: *mut u16, n: u32, read: *mut u32, control: *const u8) -> i32;
    fn WriteConsoleW(h: Handle, buf: *const u16, n: u32, written: *mut u32, reserved: *const u8) -> i32;
    fn ReadConsoleInputW(h: Handle, buf: *mut u8, n: u32, read: *mut u32) -> i32;
    fn PeekConsoleInputW(h: Handle, buf: *mut u8, n: u32, read: *mut u32) -> i32;
    fn GetNumberOfConsoleInputEvents(h: Handle, n: *mut u32) -> i32;
    fn FlushConsoleInputBuffer(h: Handle) -> i32;
    fn GetConsoleScreenBufferInfo(h: Handle, info: *mut u8) -> i32;
    fn GetConsoleScreenBufferInfoEx(h: Handle, info: *mut u8) -> i32;
    fn SetConsoleTextAttribute(h: Handle, attr: u16) -> i32;
    fn SetConsoleCursorPosition(h: Handle, at: u32) -> i32;
    fn FillConsoleOutputCharacterW(h: Handle, ch: u16, n: u32, at: u32, written: *mut u32) -> i32;
    fn FillConsoleOutputAttribute(h: Handle, attr: u16, n: u32, at: u32, written: *mut u32) -> i32;
    fn GetConsoleCP() -> u32;
    fn SetConsoleCP(cp: u32) -> i32;
    fn GetConsoleOutputCP() -> u32;
    fn SetConsoleOutputCP(cp: u32) -> i32;
    fn SetConsoleCtrlHandler(routine: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
    fn GetConsoleWindow() -> Handle;
    fn SetConsoleTitleW(title: *const u16) -> i32;
    fn GetConsoleTitleW(buf: *mut u16, size: u32) -> u32;
}

/// The exit status when every check passed.
const PASS: u32 = 0x4C;

const STD_INPUT_HANDLE: u32 = -10i32 as u32;
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
const STD_ERROR_HANDLE: u32 = -12i32 as u32;
const FILE_TYPE_DISK: u32 = 1;
const FILE_TYPE_CHAR: u32 = 2;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const OPEN_EXISTING: u32 = 3;
const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
const CTRL_C_EVENT: u32 = 0;
const KEY_EVENT: u16 = 1;

const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INVALID_PARAMETER: u32 = 87;

/// The console's standard error: where reports go.
static ERR: AtomicIsize = AtomicIsize::new(0);
static STEP: AtomicU32 = AtomicU32::new(0);
static CTRL_EVENT: AtomicU32 = AtomicU32::new(u32::MAX);
static CTRL_DONE: AtomicIsize = AtomicIsize::new(0);

fn say(bytes: &[u8]) {
    let h = ERR.load(Ordering::Relaxed);
    let mut n = 0;
    // SAFETY: a valid buffer.
    unsafe { WriteFile(h, bytes.as_ptr(), bytes.len() as u32, &mut n, core::ptr::null_mut()) };
}

fn say_num(mut v: u32) {
    let mut b = [0u8; 10];
    let mut i = b.len();
    loop {
        i -= 1;
        b[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    say(&b[i..]);
}

fn check(ok: bool) {
    let step = STEP.fetch_add(1, Ordering::Relaxed) + 1;
    if !ok {
        // SAFETY: plain kernel32 calls.
        let e = unsafe { GetLastError() };
        say(b"contest: step ");
        say_num(step);
        say(b" failed (last error ");
        say_num(e);
        say(b")\r\n");
        // SAFETY: never returns.
        unsafe { ExitProcess(0x80 + (step & 0x7F)) }
    }
}

fn check_error(want: u32) {
    // SAFETY: a plain kernel32 call.
    check(unsafe { GetLastError() } == want);
}

/// `s` as UTF-16 with a NUL, in `buf`; the units before the NUL.
fn wide<'a>(s: &str, buf: &'a mut [u16; 128]) -> &'a [u16] {
    let mut n = 0;
    for u in s.encode_utf16() {
        buf[n] = u;
        n += 1;
    }
    buf[n] = 0;
    &buf[..n]
}

fn write_w(h: Handle, s: &str) -> (i32, u32) {
    let mut b = [0u16; 128];
    let w = wide(s, &mut b);
    let mut n = 0;
    // SAFETY: a valid buffer.
    let ok = unsafe { WriteConsoleW(h, w.as_ptr(), w.len() as u32, &mut n, core::ptr::null()) };
    (ok, n)
}

const fn coord(x: u16, y: u16) -> u32 {
    (x as u32) | ((y as u32) << 16)
}

/// `CONSOLE_SCREEN_BUFFER_INFO`: (size, cursor, attributes).
type Info = ((u16, u16), (u16, u16), u16);

fn info(h: Handle) -> Option<Info> {
    let mut b = [0u8; 22];
    // SAFETY: a 22-byte structure.
    if unsafe { GetConsoleScreenBufferInfo(h, b.as_mut_ptr()) } == 0 {
        return None;
    }
    let h16 = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    Some(((h16(0), h16(2)), (h16(4), h16(6)), h16(8)))
}

unsafe extern "system" fn on_ctrl(event: u32) -> i32 {
    CTRL_EVENT.store(event, Ordering::SeqCst);
    // SAFETY: the event made before the handler was registered.
    unsafe { SetEvent(CTRL_DONE.load(Ordering::SeqCst)) };
    1
}

unsafe fn handles_and_output() {
    // SAFETY (whole function): kernel32 calls with valid arguments.
    unsafe {
        let hin = GetStdHandle(STD_INPUT_HANDLE);
        let hout = GetStdHandle(STD_OUTPUT_HANDLE);
        let herr = GetStdHandle(STD_ERROR_HANDLE);
        ERR.store(herr, Ordering::Relaxed);
        // Input and error are the console; output is redirected to a file.
        check(hin != 0 && hout != 0 && herr != 0 && hout != herr);
        check(GetFileType(hin) == FILE_TYPE_CHAR);
        check(GetFileType(herr) == FILE_TYPE_CHAR);
        check(GetFileType(hout) == FILE_TYPE_DISK);
        let mut mode = 0;
        check(GetConsoleMode(hin, &mut mode) != 0 && mode == 0x1F7);
        check(GetConsoleMode(herr, &mut mode) != 0 && mode == 3);
        check(GetConsoleMode(hout, &mut mode) == 0);
        check_error(ERROR_INVALID_HANDLE);
        // WriteConsoleW fails on the redirected handle; WriteFile works.
        check(write_w(hout, "not here").0 == 0);
        check_error(ERROR_INVALID_HANDLE);
        let mut n = 0;
        check(WriteFile(hout, b"redirected\r\n".as_ptr(), 12, &mut n, core::ptr::null_mut()) != 0 && n == 12);

        // UTF-16 to the console.
        check(write_w(herr, "Hello, W\u{f6}rld \u{2713}\r\n") == (1, 16));
        // Code pages: 437 first, then UTF-8 for WriteFile.
        check(GetConsoleOutputCP() == 437 && GetConsoleCP() == 437);
        check(SetConsoleOutputCP(12345) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        check(SetConsoleOutputCP(65001) != 0 && GetConsoleOutputCP() == 65001);
        check(SetConsoleCP(65001) != 0 && GetConsoleCP() == 65001);
        let text = "UTF-8: \u{fc}n\u{ef}code\r\n".as_bytes();
        check(WriteFile(herr, text.as_ptr(), text.len() as u32, &mut n, core::ptr::null_mut()) != 0);
        check(n as usize == text.len());

        // The screen buffer.
        check(info(herr) == Some(((80, 25), (0, 2), 7)));
        check(SetConsoleTextAttribute(herr, 0x0A) != 0);
        check(write_w(herr, "green").0 != 0);
        check(SetConsoleTextAttribute(herr, 7) != 0);
        check(SetConsoleCursorPosition(herr, coord(5, 10)) != 0);
        check(write_w(herr, "@").0 != 0);
        check(info(herr) == Some(((80, 25), (6, 10), 7)));
        check(SetConsoleCursorPosition(herr, coord(80, 0)) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        check(FillConsoleOutputCharacterW(herr, u16::from(b'*'), 10, coord(0, 12), &mut n) != 0 && n == 10);
        check(FillConsoleOutputAttribute(herr, 0x1F, 10, coord(0, 12), &mut n) != 0 && n == 10);
        let mut ex = [0u8; 96];
        check(GetConsoleScreenBufferInfoEx(herr, ex.as_mut_ptr()) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        ex[0] = 96;
        check(GetConsoleScreenBufferInfoEx(herr, ex.as_mut_ptr()) != 0);
        check(u16::from_le_bytes([ex[4], ex[5]]) == 80 && u16::from_le_bytes([ex[8], ex[9]]) == 6);
        check(GetConsoleScreenBufferInfo(hin, ex.as_mut_ptr()) == 0);
        check_error(ERROR_INVALID_HANDLE);

        // Modes: VT processing on the output, bad input modes refused.
        check(SetConsoleCursorPosition(herr, coord(0, 14)) != 0);
        check(SetConsoleMode(herr, 7) != 0 && GetConsoleMode(herr, &mut mode) != 0 && mode == 7);
        check(write_w(herr, "\x1b[31mVT\x1b[0m\r\n").0 != 0);
        check(SetConsoleMode(herr, 3) != 0);
        check(SetConsoleMode(hin, 4) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        check(SetConsoleMode(hin, 0xFFFF) == 0);
        check_error(ERROR_INVALID_PARAMETER);

        // The title and the window.
        let mut b = [0u16; 128];
        check(SetConsoleTitleW(wide("Poly console test", &mut b).as_ptr()) != 0);
        let mut t = [0u16; 64];
        let n = GetConsoleTitleW(t.as_mut_ptr(), 64) as usize;
        check(n == 17 && t[..n] == *wide("Poly console test", &mut b) && t[n] == 0);
        check(GetConsoleWindow() == 0);
    }
}

unsafe fn input() {
    // SAFETY (whole function): kernel32 calls with valid arguments.
    unsafe {
        let hin = GetStdHandle(STD_INPUT_HANDLE);
        let herr = ERR.load(Ordering::Relaxed);
        check(SetConsoleCursorPosition(herr, coord(0, 16)) != 0);
        say(b"READY-INPUT\r\n");
        // A cooked line, echoed.
        let mut buf = [0u16; 64];
        let mut n = 0;
        check(ReadConsoleW(hin, buf.as_mut_ptr(), 64, &mut n, core::ptr::null()) != 0);
        let mut b = [0u16; 128];
        check(n == 7 && buf[..7] == *wide("hello\r\n", &mut b));
        // ReadFile on the console: a line edited with backspace, in the
        // input code page (UTF-8 since above).
        let mut bytes = [0u8; 64];
        check(ReadFile(hin, bytes.as_mut_ptr(), 64, &mut n, core::ptr::null_mut()) != 0);
        check(&bytes[..n as usize] == b"abd\r\n");
        // Raw: the records left. A cooked read ends at Enter's press, so
        // its release is still there, then the typed-ahead key.
        check(SetConsoleMode(hin, 0) != 0);
        let mut count = 0;
        check(GetNumberOfConsoleInputEvents(hin, &mut count) != 0 && count == 3);
        let mut rec = [0u8; 80];
        check(PeekConsoleInputW(hin, rec.as_mut_ptr(), 4, &mut n) != 0 && n == 3);
        check(u16::from_le_bytes([rec[0], rec[1]]) == KEY_EVENT && rec[4] == 0 && rec[10] == 0x0D);
        check(rec[20] == 1 && rec[24] == 1 && rec[34] == b'x' && rec[44] == 0);
        check(ReadConsoleInputW(hin, rec.as_mut_ptr(), 2, &mut n) != 0 && n == 2 && rec[34] == b'x');
        check(GetNumberOfConsoleInputEvents(hin, &mut count) != 0 && count == 1);
        check(FlushConsoleInputBuffer(hin) != 0);
        check(GetNumberOfConsoleInputEvents(hin, &mut count) != 0 && count == 0);
        check(SetConsoleMode(hin, 0x1F7) != 0);

        // Ctrl+C reaches the handler, on a thread of its own.
        let ev = CreateEventW(core::ptr::null(), 1, 0, core::ptr::null());
        check(ev != 0);
        CTRL_DONE.store(ev, Ordering::SeqCst);
        check(SetConsoleCtrlHandler(Some(on_ctrl), 1) != 0);
        say(b"READY-CTRLC\r\n");
        check(WaitForSingleObject(ev, 20_000) == 0);
        check(CTRL_EVENT.load(Ordering::SeqCst) == CTRL_C_EVENT);
        check(SetConsoleCtrlHandler(Some(on_ctrl), 0) != 0);
        check(SetConsoleCtrlHandler(Some(on_ctrl), 0) == 0);
        check_error(ERROR_INVALID_PARAMETER);
    }
}

unsafe fn conout_and_attaching() {
    // SAFETY (whole function): kernel32 calls with valid arguments.
    unsafe {
        let mut b = [0u16; 128];
        let con = CreateFileW(
            wide("CONOUT$", &mut b).as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            3,
            core::ptr::null(),
            OPEN_EXISTING,
            0,
            0,
        );
        check(con != -1 && GetFileType(con) == FILE_TYPE_CHAR);
        check(write_w(con, "via CONOUT$\r\n") == (1, 13));
        check(CloseHandle(con) != 0);
        say(b"contest: console checks passed\r\n");

        // Leaving the console closes it (nobody else is on it).
        check(FreeConsole() != 0);
        SetLastError(0);
        check(GetConsoleCP() == 0);
        check_error(ERROR_INVALID_HANDLE);
        check(AttachConsole(ATTACH_PARENT_PROCESS) == 0);
        check_error(ERROR_INVALID_HANDLE);
        check(FreeConsole() == 0);
        // A new console: the standard handles move to it.
        check(AllocConsole() != 0);
        check(AllocConsole() == 0);
        check_error(ERROR_ACCESS_DENIED);
        check(AttachConsole(ATTACH_PARENT_PROCESS) == 0);
        check_error(ERROR_ACCESS_DENIED);
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        check(GetFileType(out) == FILE_TYPE_CHAR && GetConsoleMode(out, &mut mode) != 0 && mode == 3);
        check(GetConsoleOutputCP() == 437);
        check(FreeConsole() != 0);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn contest_main(_peb: *mut u8) -> u32 {
    // SAFETY: kernel32 calls with valid arguments.
    unsafe {
        handles_and_output();
        input();
        conout_and_attaching();
        ExitProcess(PASS)
    }
}
