//! Handle inheritance, STARTUPINFO and CREATE_SUSPENDED (ADR 0089).

use super::*;

// ----- handle inheritance, STARTUPINFO, CREATE_SUSPENDED (ADR 0089) --------------------

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetHandleInformation(h: Handle, flags: *mut u32) -> i32;
    fn SetHandleInformation(h: Handle, mask: u32, flags: u32) -> i32;
    fn DuplicateHandle(sp: Handle, s: Handle, tp: Handle, t: *mut Handle, access: u32, inherit: i32, opts: u32) -> i32;
    fn CreateProcessA(
        app: *const u8,
        cmd: *mut u8,
        pa: *const u8,
        ta: *const u8,
        inherit: i32,
        flags: u32,
        env: *const u8,
        cwd: *const u8,
        si: *const u8,
        pi: *mut u8,
    ) -> i32;
    fn GetStartupInfoA(info: *mut u8);
    fn GetFileType(h: Handle) -> u32;
    fn GetConsoleMode(h: Handle, mode: *mut u32) -> i32;
}

/// `HANDLE_FLAG_*` (`handleapi.h`), `DUPLICATE_SAME_ACCESS` (`winnt.h`),
/// `STARTF_USESTDHANDLES` (`processthreadsapi.h`), `CREATE_SUSPENDED`
/// (`winbase.h`), the standard handle selectors (`processenv.h`),
/// `FILE_TYPE_DISK` (`fileapi.h`), `ERROR_INVALID_HANDLE` (`winerror.h`).
const HANDLE_FLAG_INHERIT: u32 = 1;
const HANDLE_FLAG_PROTECT_FROM_CLOSE: u32 = 2;
const DUPLICATE_SAME_ACCESS: u32 = 2;
const STARTF_USESTDHANDLES: u32 = 0x100;
const CREATE_SUSPENDED: u32 = 4;
const STD_INPUT_HANDLE: u32 = -10i32 as u32;
const STD_ERROR_HANDLE: u32 = -12i32 as u32;
const FILE_TYPE_DISK: u32 = 1;
const ERROR_INVALID_HANDLE: u32 = 6;
/// `FILE_SHARE_READ | FILE_SHARE_WRITE`.
const SHARE_READ_WRITE: u32 = 3;
const CHILD_STDOUT: &[u8] = b"child stdout\r\n";
const CHILD_STDERR: &[u8] = b"child stderr\r\n";

/// `s` (ASCII) appended to `buf` at `*at`.
pub(crate) fn push_ascii(buf: &mut [u16], at: &mut usize, s: &[u8]) {
    for &c in s {
        buf[*at] = u16::from(c);
        *at += 1;
    }
}

/// `v` in decimal appended to `buf` at `*at`.
fn push_num(buf: &mut [u16], at: &mut usize, v: u64) {
    let mut digits = [0u8; 20];
    let (mut i, mut v) = (digits.len(), v);
    loop {
        i -= 1;
        digits[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    push_ascii(buf, at, &digits[i..]);
}

/// `dir` (ending in a separator) and `name`, NUL-terminated.
fn path_in(dir: &[u16], name: &[u8]) -> [u16; 300] {
    let mut p = [0u16; 300];
    p[..dir.len()].copy_from_slice(dir);
    let mut at = dir.len();
    push_ascii(&mut p, &mut at, name);
    p
}

/// The `index`th decimal number after the `--childN` flag.
fn arg_num(cmd: &[u16], index: usize) -> u64 {
    let start = cmd.windows(7).position(|w| ascii_eq(w, b"--child")).unwrap_or(0);
    let mut words = cmd[start..].split(|&c| c == u16::from(b' ')).filter(|w| !w.is_empty()).skip(1);
    let word = words.nth(index).unwrap_or(&[]);
    word.iter().fold(0u64, |n, &c| n * 10 + u64::from(c.wrapping_sub(u16::from(b'0')) & 0xF))
}

/// The first bytes of the file at `path` (up to 64).
///
/// # Safety
/// `path` is NUL-terminated.
pub(super) unsafe fn read_back(path: &[u16]) -> ([u8; 64], usize) {
    let mut buf = [0u8; 64];
    let mut n = 0u32;
    // SAFETY: valid arguments.
    unsafe {
        let h = CreateFileW(path.as_ptr(), GENERIC_READ, SHARE_READ_WRITE, np(), OPEN_EXISTING, 0, 0);
        if h != INVALID_HANDLE_VALUE {
            ReadFile(h, buf.as_mut_ptr(), 64, &mut n, core::ptr::null_mut());
            CloseHandle(h);
        }
    }
    (buf, n as usize)
}

pub(super) unsafe fn inheritance_and_startup() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let mut temp = [0u16; 260];
        let n = GetTempPathW(260, temp.as_mut_ptr()) as usize;
        check(n > 3);
        let out_path = path_in(&temp[..n], b"k32-child-out.txt");
        let err_path = path_in(&temp[..n], b"k32-child-err.txt");
        // Inheritable through SECURITY_ATTRIBUTES.bInheritHandle.
        let mut sa = [0u8; 24];
        sa[0..4].copy_from_slice(&24u32.to_le_bytes());
        sa[16..20].copy_from_slice(&1i32.to_le_bytes());
        let out_h =
            CreateFileW(out_path.as_ptr(), GENERIC_WRITE, SHARE_READ_WRITE, sa.as_ptr(), CREATE_ALWAYS, 0x80, 0);
        check(out_h != INVALID_HANDLE_VALUE);
        let mut f = 0xFFu32;
        check(GetHandleInformation(out_h, &mut f) != 0 && f == HANDLE_FLAG_INHERIT);
        // Made inheritable afterwards; protected from close for a while.
        let err_h = CreateFileW(err_path.as_ptr(), GENERIC_WRITE, SHARE_READ_WRITE, np(), CREATE_ALWAYS, 0x80, 0);
        check(err_h != INVALID_HANDLE_VALUE);
        check(GetHandleInformation(err_h, &mut f) != 0 && f == 0);
        check(SetHandleInformation(err_h, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) != 0);
        check(GetHandleInformation(err_h, &mut f) != 0 && f == HANDLE_FLAG_INHERIT);
        let protect = HANDLE_FLAG_PROTECT_FROM_CLOSE;
        check(SetHandleInformation(err_h, protect, protect | HANDLE_FLAG_INHERIT) != 0);
        check(GetHandleInformation(err_h, &mut f) != 0 && f == HANDLE_FLAG_INHERIT | protect);
        check(CloseHandle(err_h) == 0);
        check(SetHandleInformation(err_h, protect, 0) != 0);
        check(GetHandleInformation(err_h, &mut f) != 0 && f == HANDLE_FLAG_INHERIT);
        check(SetHandleInformation(err_h, 4, 4) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        check(GetHandleInformation(0x7FFC, &mut f) == 0);
        check_error(ERROR_INVALID_HANDLE);
        // Handles the child must not get: enough of them that the last
        // one's value is above any the child opens for itself.
        let mut private = [0 as Handle; 48];
        for p in private.iter_mut() {
            *p = CreateEventW(np(), 1, 0, np());
            check(*p != 0);
        }
        let hidden = private[47];
        check(GetHandleInformation(hidden, &mut f) != 0 && f == 0);
        // An inheritable copy of one of them: the same event in the child.
        let mut dup: Handle = 0;
        let me = GetCurrentProcess();
        check(DuplicateHandle(me, private[0], me, &mut dup, 0, 1, DUPLICATE_SAME_ACCESS) != 0);
        check(dup != 0 && dup != private[0]);
        check(GetHandleInformation(dup, &mut f) != 0 && f == HANDLE_FLAG_INHERIT);
        check(GetHandleInformation(private[0], &mut f) != 0 && f == 0);

        // The child: standard output and error redirected to the files,
        // its standard input the creator's, a title and a position, created
        // suspended, inheriting.
        let mut si = [0u8; 104];
        si[0..4].copy_from_slice(&104u32.to_le_bytes());
        si[24..32].copy_from_slice(&(w!("poly child") as u64).to_le_bytes());
        si[32..36].copy_from_slice(&0x11u32.to_le_bytes());
        si[60..64].copy_from_slice(&STARTF_USESTDHANDLES.to_le_bytes());
        let input = GetStdHandle(STD_INPUT_HANDLE);
        for (i, h) in [input, out_h, err_h].iter().enumerate() {
            si[80 + 8 * i..88 + 8 * i].copy_from_slice(&(*h as u64).to_le_bytes());
        }
        let mut cmd = [0u16; 128];
        let mut at = 0;
        push_ascii(&mut cmd, &mut at, b"k32test.exe --child3 ");
        for (i, v) in [dup, hidden, input].into_iter().enumerate() {
            if i > 0 {
                push_ascii(&mut cmd, &mut at, b" ");
            }
            push_num(&mut cmd, &mut at, v as u64);
        }
        let mut pi = [0u8; 24];
        let flags = CREATE_SUSPENDED;
        check(
            CreateProcessW(np(), cmd.as_mut_ptr(), np(), np(), 1, flags, np(), np(), si.as_ptr(), pi.as_mut_ptr()) != 0,
        );
        let ph = isize::from_le_bytes(pi[0..8].try_into().unwrap());
        let th = isize::from_le_bytes(pi[8..16].try_into().unwrap());
        check(ph != 0 && th != 0 && u32::from_le_bytes(pi[20..24].try_into().unwrap()) != 0);
        // Suspended: it has not run yet.
        check(WaitForSingleObject(ph, 300) == WAIT_TIMEOUT);
        let mut code = 0u32;
        check(GetExitCodeProcess(ph, &mut code) != 0 && code == STILL_ACTIVE);
        check(GetFileSize(out_h, core::ptr::null_mut()) == 0);
        check(WaitForSingleObject(private[0], 0) == WAIT_TIMEOUT);
        check(ResumeThread(th) == 1);
        check(WaitForSingleObject(ph, 30_000) == WAIT_OBJECT_0);
        check(GetExitCodeProcess(ph, &mut code) != 0);
        if code != 0xC0DE_0003 {
            // Its report went to its standard output: the file.
            out(b"k32test: --child3 ended with ");
            out_num(code);
            out(b": ");
            let (b, n) = read_back(&out_path);
            out(&b[..n]);
            out(b"\n");
        }
        check(code == 0xC0DE_0003);
        // It set the event through its inherited copy.
        check(WaitForSingleObject(private[0], 0) == WAIT_OBJECT_0);
        check(CloseHandle(th) != 0 && CloseHandle(ph) != 0);
        // It wrote the files through the inherited handles.
        let (b, n) = read_back(&out_path);
        check(&b[..n] == CHILD_STDOUT);
        let (b, n) = read_back(&err_path);
        check(&b[..n] == CHILD_STDERR);
        check(CloseHandle(out_h) != 0 && CloseHandle(err_h) != 0 && CloseHandle(dup) != 0);
        for p in private {
            check(CloseHandle(p) != 0);
        }
        check(DeleteFileW(out_path.as_ptr()) != 0 && DeleteFileW(err_path.as_ptr()) != 0);

        // CreateProcessA: an ANSI environment block and a STARTUPINFOA.
        let mut si = [0u8; 104];
        si[0..4].copy_from_slice(&104u32.to_le_bytes());
        si[24..32].copy_from_slice(&(c"ansi title".as_ptr() as u64).to_le_bytes());
        let mut cmd = *b"k32test.exe --child4\0";
        let env = b"POLY_CHILD=ansi\0Path=C:\\Windows\\System32\0\0";
        check(
            CreateProcessA(np(), cmd.as_mut_ptr(), np(), np(), 0, 0, env.as_ptr(), np(), si.as_ptr(), pi.as_mut_ptr())
                != 0,
        );
        let ph = isize::from_le_bytes(pi[0..8].try_into().unwrap());
        let th = isize::from_le_bytes(pi[8..16].try_into().unwrap());
        check(WaitForSingleObject(ph, 30_000) == WAIT_OBJECT_0);
        check(GetExitCodeProcess(ph, &mut code) != 0 && code == 0xC0DE_0004);
        check(CloseHandle(th) != 0 && CloseHandle(ph) != 0);
    }
}

/// `--child3`: started suspended by `inheritance_and_startup` with
/// redirected standard output and error, inheriting; the numbers after the
/// flag are an inherited event, a handle it must not have, and its
/// creator's standard input.
pub(super) unsafe fn child3(cmd: &[u16]) -> ! {
    // SAFETY: valid arguments throughout.
    unsafe {
        let (dup, hidden, input) = (arg_num(cmd, 0) as Handle, arg_num(cmd, 1) as Handle, arg_num(cmd, 2) as Handle);
        let out_h = GetStdHandle(STD_OUTPUT_HANDLE);
        let err_h = GetStdHandle(STD_ERROR_HANDLE);
        check(out_h != 0 && out_h != INVALID_HANDLE_VALUE && err_h != 0 && err_h != out_h);
        // A file, not a console: 7-Zip asks.
        check(GetFileType(out_h) == FILE_TYPE_DISK && GetFileType(err_h) == FILE_TYPE_DISK);
        let mut mode = 0u32;
        check(GetConsoleMode(out_h, &mut mode) == 0);
        check_error(ERROR_INVALID_HANDLE);
        let mut n = 0u32;
        check(WriteFile(out_h, CHILD_STDOUT.as_ptr(), CHILD_STDOUT.len() as u32, &mut n, core::ptr::null_mut()) != 0);
        check(n == CHILD_STDOUT.len() as u32);
        check(WriteFile(err_h, CHILD_STDERR.as_ptr(), CHILD_STDERR.len() as u32, &mut n, core::ptr::null_mut()) != 0);
        // The STARTUPINFO its creator passed.
        let mut si = [0xFFu8; 104];
        GetStartupInfoW(si.as_mut_ptr());
        let u32_at = |o: usize| u32::from_le_bytes(si[o..o + 4].try_into().unwrap());
        let u64_at = |o: usize| u64::from_le_bytes(si[o..o + 8].try_into().unwrap());
        check(u32_at(0) == 104 && u32_at(60) == STARTF_USESTDHANDLES && u32_at(32) == 0x11);
        check(u64_at(16) == 0 && u64_at(24) != 0 && ascii_eq(wslice(u64_at(24) as *const u16), b"poly child"));
        check(u64_at(80) as Handle == input && u64_at(88) as Handle == out_h && u64_at(96) as Handle == err_h);
        // Its standard input: the creator's, inherited.
        check(GetStdHandle(STD_INPUT_HANDLE) == input);
        check(input == 0 || GetFileType(input) != 0);
        // The inherited copy is its creator's event; the other handle is
        // not there.
        check(SetEvent(dup) != 0);
        let mut f = 0u32;
        check(GetHandleInformation(hidden, &mut f) == 0);
        check_error(ERROR_INVALID_HANDLE);
        ExitProcess(0xC0DE_0003)
    }
}

/// `--child4`: started by `CreateProcessA` with an ANSI environment block
/// and a `STARTUPINFOA`.
pub(super) unsafe fn child4() -> ! {
    // SAFETY: valid arguments throughout.
    unsafe {
        let mut v = [0u16; 64];
        let k = GetEnvironmentVariableW(w!("POLY_CHILD"), v.as_mut_ptr(), 64) as usize;
        check(ascii_eq(&v[..k], b"ansi"));
        let mut si = [0u8; 104];
        GetStartupInfoA(si.as_mut_ptr());
        let title = u64::from_le_bytes(si[24..32].try_into().unwrap()) as *const u8;
        check(!title.is_null());
        let mut len = 0;
        while *title.add(len) != 0 {
            len += 1;
        }
        check(core::slice::from_raw_parts(title, len) == b"ansi title");
        GetStartupInfoW(si.as_mut_ptr());
        let title = u64::from_le_bytes(si[24..32].try_into().unwrap()) as *const u16;
        check(!title.is_null() && ascii_eq(wslice(title), b"ansi title"));
        check(si[60..64] == [0; 4]);
        ExitProcess(0xC0DE_0004)
    }
}
