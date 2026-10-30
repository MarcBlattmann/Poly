//! `crtiotest.exe` (ADR 0093): the program the kernel's ring-3 test
//! `nt_c_runtime_files_processes_and_console` runs. Like `crttest.exe` it
//! imports the UCRT through `api-ms-win-crt-*` names and does MSVC's
//! start-up itself, then checks this slice of the runtime: `_findfirst` /
//! `_findnext`, `_dup` / `_dup2` / `_pipe`, `_spawn*` / `_exec*` / `_cwait`
//! / `system` / `_popen` (running itself as the child and, through
//! `COMSPEC`, as the command interpreter), wildcard `argv`, and console
//! I/O (keys written into its own console's input buffer, output measured
//! by the cursor), plus `msvcrt.dll`'s forms through `GetProcAddress`.
//!
//! The first argument picks the role: none or `--poly` runs the checks
//! and ends with `exit(PASS)`; the others are the children's (below). A
//! failed check prints `crtiotest: step N failed` and exits with
//! `0x80 + N`.

#![no_std]
#![no_main]
#![allow(clippy::missing_safety_doc)]

use core::sync::atomic::{AtomicU32, Ordering};
use poly_win32_rt as _;

/// The exit status when every check passed.
const PASS: i32 = 0x44;
/// Where the kernel test installs the program.
const APP_DIR: &core::ffi::CStr = c"C:\\Program Files\\PolyCrtIoTest";

type File = u8;

#[link(name = "api-ms-win-crt-runtime-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn _set_app_type(t: i32);
    fn _configure_narrow_argv(mode: i32) -> i32;
    fn _configure_wide_argv(mode: i32) -> i32;
    fn _initialize_narrow_environment() -> i32;
    fn __p___argc() -> *mut i32;
    fn __p___argv() -> *mut *mut *mut u8;
    fn __p___wargv() -> *mut *mut *mut u16;
    fn _set_invalid_parameter_handler(h: extern "C" fn(*const u16, *const u16, *const u16, u32, usize)) -> usize;
    fn _errno() -> *mut i32;
    fn exit(code: i32) -> !;
    fn system(c: *const u8) -> i32;
    fn _wsystem(c: *const u16) -> i32;
}

#[link(name = "api-ms-win-crt-process-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn _spawnl(mode: i32, cmd: *const u8, arg0: *const u8, ...) -> isize;
    fn _spawnle(mode: i32, cmd: *const u8, arg0: *const u8, ...) -> isize;
    fn _spawnlp(mode: i32, cmd: *const u8, arg0: *const u8, ...) -> isize;
    fn _spawnv(mode: i32, cmd: *const u8, argv: *const *const u8) -> isize;
    fn _wspawnv(mode: i32, cmd: *const u16, argv: *const *const u16) -> isize;
    fn _execl(cmd: *const u8, arg0: *const u8, ...) -> isize;
    fn _cwait(status: *mut i32, h: isize, action: i32) -> isize;
}

#[link(name = "api-ms-win-crt-stdio-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn __acrt_iob_func(i: u32) -> *mut File;
    fn fflush(f: *mut File) -> i32;
    fn fputs(s: *const u8, f: *mut File) -> i32;
    fn fgets(b: *mut u8, n: i32, f: *mut File) -> *mut u8;
    fn puts(s: *const u8) -> i32;
    fn _open(n: *const u8, flags: i32, pmode: i32) -> i32;
    fn _read(fd: i32, b: *mut u8, n: u32) -> i32;
    fn _write(fd: i32, b: *const u8, n: u32) -> i32;
    fn _close(fd: i32) -> i32;
    fn _lseek(fd: i32, o: i32, w: i32) -> i32;
    fn _dup(fd: i32) -> i32;
    fn _dup2(a: i32, b: i32) -> i32;
    fn _pipe(fds: *mut i32, size: u32, mode: i32) -> i32;
    fn _get_osfhandle(fd: i32) -> isize;
    fn _popen(c: *const u8, m: *const u8) -> *mut File;
    fn _pclose(f: *mut File) -> i32;
}

#[link(name = "api-ms-win-crt-filesystem-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn _findfirst64(p: *const u8, d: *mut u8) -> isize;
    fn _findnext64(h: isize, d: *mut u8) -> i32;
    fn _findfirst32(p: *const u8, d: *mut u8) -> isize;
    fn _findfirst32i64(p: *const u8, d: *mut u8) -> isize;
    fn _wfindfirst64(p: *const u16, d: *mut u8) -> isize;
    fn _findclose(h: isize) -> i32;
    fn _mkdir(n: *const u8) -> i32;
    fn _chdir(n: *const u8) -> i32;
}

#[link(name = "api-ms-win-crt-conio-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn _getch() -> i32;
    fn _getche() -> i32;
    fn _getwch() -> u16;
    fn _kbhit() -> i32;
    fn _ungetch(c: i32) -> i32;
    fn _putch(c: i32) -> i32;
    fn _putwch(c: u16) -> u16;
    fn _cputs(s: *const u8) -> i32;
    fn _cputws(s: *const u16) -> i32;
    fn _cgets(b: *mut u8) -> *mut u8;
    fn _cgets_s(b: *mut u8, n: usize, read: *mut usize) -> i32;
    fn _cgetws(b: *mut u16) -> *mut u16;
    fn __conio_common_vcprintf(o: u64, f: *const u8, l: *mut u8, va: *const u64) -> i32;
    fn __conio_common_vcwprintf(o: u64, f: *const u16, l: *mut u8, va: *const u64) -> i32;
}

#[link(name = "api-ms-win-crt-environment-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn getenv(n: *const u8) -> *const u8;
    fn _putenv(s: *const u8) -> i32;
}

#[link(name = "api-ms-win-crt-string-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn atoi(s: *const u8) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ExitProcess(code: u32) -> !;
    fn GetStdHandle(which: u32) -> isize;
    fn WriteFile(h: isize, b: *const u8, n: u32, w: *mut u32, o: *mut u8) -> i32;
    fn LoadLibraryW(n: *const u16) -> *mut u8;
    fn GetProcAddress(m: *mut u8, n: *const u8) -> *mut u8;
    fn GetFileType(h: isize) -> u32;
    fn CreateFileW(n: *const u16, a: u32, s: u32, sa: *const u8, d: u32, f: u32, t: isize) -> isize;
    fn WriteConsoleInputW(h: isize, r: *const u8, n: u32, w: *mut u32) -> i32;
    fn GetConsoleScreenBufferInfo(h: isize, info: *mut u8) -> i32;
    fn SetConsoleCursorPosition(h: isize, pos: u32) -> i32;
    fn FlushConsoleInputBuffer(h: isize) -> i32;
    fn Sleep(ms: u32);
}

/// The UCRT headers' default options (`corecrt_stdio_config.h`).
const OPTS: u64 = 4 | 32;
const EBADF: i32 = 9;
const ENOENT: i32 = 2;
const EINVAL: i32 = 22;
/// `fcntl.h`, `sys/stat.h`, `process.h`, `corecrt_io.h`.
const O_WRONLY: i32 = 0x1;
const O_RDWR: i32 = 0x2;
const O_NOINHERIT: i32 = 0x80;
const O_CREAT: i32 = 0x100;
const O_TRUNC: i32 = 0x200;
const O_TEXT: i32 = 0x4000;
const O_BINARY: i32 = 0x8000;
const PMODE: i32 = 0x100 | 0x80;
const P_WAIT: i32 = 0;
const P_NOWAIT: i32 = 1;
const A_SUBDIR: u32 = 0x10;

static STEP: AtomicU32 = AtomicU32::new(0);
static INVALID: AtomicU32 = AtomicU32::new(0);

fn say(text: &[u8]) {
    let mut n = 0;
    // SAFETY: kernel32 with a valid buffer.
    unsafe {
        WriteFile(GetStdHandle((-11i32) as u32), text.as_ptr(), text.len() as u32, &mut n, core::ptr::null_mut())
    };
}

fn fail(step: u32) -> ! {
    let mut msg = *b"crtiotest: step 000 failed\r\n";
    msg[16] = b'0' + (step / 100 % 10) as u8;
    msg[17] = b'0' + (step / 10 % 10) as u8;
    msg[18] = b'0' + (step % 10) as u8;
    say(&msg);
    // SAFETY: no preconditions.
    unsafe { ExitProcess(0x80 + step) }
}

/// The next step: fail it unless `ok`.
fn check(ok: bool) {
    let step = STEP.fetch_add(1, Ordering::Relaxed) + 1;
    if !ok {
        fail(step);
    }
}

/// A C string's bytes (without the NUL).
fn cs<'a>(p: *const u8) -> &'a [u8] {
    if p.is_null() {
        return &[];
    }
    let mut n = 0;
    // SAFETY: a C string from the runtime.
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: n bytes are readable.
    unsafe { core::slice::from_raw_parts(p, n) }
}

/// A wide C string's units.
fn wcs<'a>(p: *const u16) -> &'a [u16] {
    if p.is_null() {
        return &[];
    }
    let mut n = 0;
    // SAFETY: a C string from the runtime.
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: n units are readable.
    unsafe { core::slice::from_raw_parts(p, n) }
}

fn ws<const N: usize>(s: &[u8; N]) -> [u16; N] {
    let mut w = [0u16; N];
    for (i, &b) in s.iter().enumerate() {
        w[i] = u16::from(b);
    }
    w
}

fn c(s: &core::ffi::CStr) -> *const u8 {
    s.as_ptr() as *const u8
}

fn errno() -> i32 {
    // SAFETY: the runtime's errno of this thread.
    unsafe { *_errno() }
}

extern "C" fn on_invalid(_e: *const u16, _f: *const u16, _file: *const u16, _l: u32, _r: usize) {
    INVALID.fetch_add(1, Ordering::Relaxed);
}

/// The whole of a file, binary.
unsafe fn read_file(name: &core::ffi::CStr) -> ([u8; 128], usize) {
    let mut buf = [0u8; 128];
    // SAFETY: the runtime with valid arguments.
    unsafe {
        let fd = _open(c(name), O_BINARY, 0);
        if fd < 0 {
            return (buf, 0);
        }
        let n = _read(fd, buf.as_mut_ptr(), 128);
        _close(fd);
        (buf, n.max(0) as usize)
    }
}

unsafe fn write_file(name: *const u8, text: &[u8]) {
    // SAFETY: the runtime with valid arguments.
    unsafe {
        let fd = _open(name, O_CREAT | O_TRUNC | O_WRONLY | O_BINARY, PMODE);
        _write(fd, text.as_ptr(), text.len() as u32);
        _close(fd);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn crtiotest_main() -> u32 {
    // SAFETY: the runtime's documented functions, each with valid arguments.
    unsafe { run() }
}

unsafe fn run() -> ! {
    // SAFETY: the whole body calls the runtime with valid arguments.
    unsafe {
        _set_app_type(1);
        if _configure_narrow_argv(1) != 0 || _initialize_narrow_environment() != 0 {
            fail(0);
        }
        _set_invalid_parameter_handler(on_invalid);
        let argc = *__p___argc() as usize;
        let argv = *__p___argv();
        let arg = |i: usize| if i < argc { cs(*argv.add(i)) } else { &[][..] };
        let role = arg(1);
        if !role.is_empty() && role != b"--poly" {
            child(role, argv, argc);
        }
        let me = *argv;
        check(_chdir(c(APP_DIR)) == 0);
        find();
        descriptors();
        processes(me);
        wildcards(me);
        console();
        msvcrt();
        puts(c(c"crtiotest: done"));
        exit(PASS)
    }
}

/// The children's roles.
unsafe fn child(role: &[u8], argv: *mut *mut u8, argc: usize) -> ! {
    // SAFETY: the runtime with valid arguments.
    unsafe {
        let arg = |i: usize| if i < argc { *argv.add(i) as *const u8 } else { core::ptr::null() };
        let code = match role {
            b"--exit" => atoi(arg(2)),
            // The arguments as _spawn joined them.
            b"--args" => {
                let ok = argc == 5 && cs(arg(2)) == b"hi" && cs(arg(3)) == b"there" && cs(arg(4)) == b"x";
                if ok { 0x21 } else { 0x71 }
            }
            // The descriptors a creator passed: one inherited, one not.
            b"--fds" => {
                let (fd, no) = (atoi(arg(2)), atoi(arg(3)));
                let wrote = _write(fd, b"inherited".as_ptr(), 9) == 9;
                let refused = _write(no, b"x".as_ptr(), 1) == -1 && errno() == EBADF;
                puts(c(c"crtiotest: child says hello"));
                if wrote && refused { 0x22 } else { 0x72 }
            }
            // Only the environment it was given.
            b"--env" => {
                let ok = cs(getenv(c(c"POLY_SPAWN"))) == b"on" && getenv(c(c"POLY_PARENT_ONLY")).is_null();
                if ok { 0x23 } else { 0x73 }
            }
            b"--echo" => {
                _write(1, b"child stdout\n".as_ptr(), 13);
                0x25
            }
            b"--exec" => {
                _execl(arg(0), arg(0), c(c"--write"), arg(2), core::ptr::null::<u8>());
                0x76
            }
            b"--write" => {
                write_file(arg(2), b"exec ok");
                0
            }
            b"--glob" => glob_child(argc),
            // The command interpreter (`COMSPEC`): `/c <command>`.
            b"/c" => match cs(arg(2)) {
                b"exit" => atoi(arg(3)),
                b"echo" => {
                    puts(arg(3));
                    0x11
                }
                b"save" => {
                    let mut buf = [0u8; 64];
                    let mut got = 0usize;
                    loop {
                        let n = _read(0, buf.as_mut_ptr().add(got), (64 - got) as u32);
                        if n <= 0 {
                            break;
                        }
                        got += n as usize;
                    }
                    write_file(arg(3), &buf[..got]);
                    0x12
                }
                _ => 0x7E,
            },
            _ => 0x7F,
        };
        exit(code)
    }
}

/// `--glob find\*.txt "find\*.dat" nomatch*.zz`: the arguments expanded.
unsafe fn glob_child(argc_plain: usize) -> i32 {
    // SAFETY: the runtime with valid arguments.
    unsafe {
        if argc_plain != 5 || _configure_narrow_argv(2) != 0 {
            return 0x70;
        }
        let argc = *__p___argc() as usize;
        let argv = *__p___argv();
        let a = |i: usize| cs(*argv.add(i));
        let names_ok = |x: &[u8], y: &[u8]| {
            (x == b"find\\a1.txt" && y == b"find\\a2.txt") || (x == b"find\\a2.txt" && y == b"find\\a1.txt")
        };
        if argc != 6 || !names_ok(a(2), a(3)) || a(4) != b"find\\*.dat" || a(5) != b"nomatch*.zz" {
            return 0x74;
        }
        if _configure_wide_argv(2) != 0 {
            return 0x75;
        }
        let w = *__p___wargv();
        if *__p___argc() != 6 || wcs(*w.add(4)) != ws(b"find\\*.dat") || wcs(*w.add(2)).len() != 11 {
            return 0x77;
        }
        // msvcrt's __getmainargs with DoWildCard.
        let m = LoadLibraryW(ws(b"msvcrt.dll\0").as_ptr());
        type GetMainArgs = unsafe extern "C" fn(*mut i32, *mut *mut *mut u8, *mut *mut *mut u8, i32, *const i32) -> i32;
        let gma: GetMainArgs = core::mem::transmute(GetProcAddress(m, c(c"__getmainargs")));
        let (mut n, mut v, mut e) = (0i32, core::ptr::null_mut(), core::ptr::null_mut());
        let newmode = 0i32;
        if gma(&mut n, &mut v, &mut e, 1, &newmode) != 0 || n != 6 || !names_ok(cs(*v.add(2)), cs(*v.add(3))) {
            return 0x78;
        }
        0x24
    }
}

/// `_findfirst` / `_findnext` / `_findclose`.
unsafe fn find() {
    // SAFETY: the runtime with valid arguments.
    unsafe {
        _mkdir(c(c"find"));
        _mkdir(c(c"find\\sub"));
        write_file(c(c"find\\a1.txt"), b"12345");
        write_file(c(c"find\\a2.txt"), b"1");
        write_file(c(c"find\\b.dat"), b"");
        // __finddata64_t: name at 40, size at 32 (304 bytes).
        let mut d = [0u8; 304];
        let h = _findfirst64(c(c"find\\*.txt"), d.as_mut_ptr());
        check(h != -1);
        let mut seen = 0;
        loop {
            let name = cs(d.as_ptr().add(40));
            let size = u64::from_le_bytes(d[32..40].try_into().unwrap());
            let attrib = u32::from_le_bytes(d[0..4].try_into().unwrap());
            match name {
                b"a1.txt" => {
                    check(size == 5 && attrib & A_SUBDIR == 0);
                    seen |= 1;
                }
                b"a2.txt" => {
                    check(size == 1);
                    seen |= 2;
                }
                _ => check(false),
            }
            if _findnext64(h, d.as_mut_ptr()) != 0 {
                break;
            }
        }
        check(seen == 3 && errno() == ENOENT);
        check(_findclose(h) == 0);
        check(_findfirst64(c(c"find\\*.zzz"), d.as_mut_ptr()) == -1 && errno() == ENOENT);
        // _finddata32_t: name at 20.
        let mut d32 = [0u8; 280];
        let h = _findfirst32(c(c"find\\sub"), d32.as_mut_ptr());
        check(
            h != -1
                && cs(d32.as_ptr().add(20)) == b"sub"
                && u32::from_le_bytes(d32[0..4].try_into().unwrap()) & A_SUBDIR != 0,
        );
        _findclose(h);
        // _finddata32i64_t: size at 16 (8 bytes), name at 24.
        let mut d3 = [0u8; 288];
        let h = _findfirst32i64(c(c"find\\a1.txt"), d3.as_mut_ptr());
        check(
            h != -1 && u64::from_le_bytes(d3[16..24].try_into().unwrap()) == 5 && cs(d3.as_ptr().add(24)) == b"a1.txt",
        );
        _findclose(h);
        // _wfinddata64_t: name at 40, wide.
        let mut wd = [0u8; 560];
        let h = _wfindfirst64(ws(b"find\\b.*\0").as_ptr(), wd.as_mut_ptr());
        check(h != -1 && wcs(wd.as_ptr().add(40) as *const u16) == ws(b"b.dat"));
        _findclose(h);
        let before = INVALID.load(Ordering::Relaxed);
        check(_findfirst64(core::ptr::null(), d.as_mut_ptr()) == -1 && errno() == EINVAL);
        check(INVALID.load(Ordering::Relaxed) == before + 1);
    }
}

/// `_dup`, `_dup2`, `_pipe`.
unsafe fn descriptors() {
    // SAFETY: the runtime with valid arguments.
    unsafe {
        let fd = _open(c(c"dup.txt"), O_CREAT | O_TRUNC | O_RDWR | O_BINARY, PMODE);
        check(fd >= 3);
        let d = _dup(fd);
        check(d > fd);
        // One file pointer for both.
        check(_write(d, b"abc".as_ptr(), 3) == 3 && _write(fd, b"def".as_ptr(), 3) == 3);
        check(_dup2(fd, 40) == 0 && _write(40, b"g".as_ptr(), 1) == 1);
        check(_close(40) == 0 && _close(d) == 0 && _close(fd) == 0);
        let (buf, n) = read_file(c"dup.txt");
        check(&buf[..n] == b"abcdefg");
        let before = INVALID.load(Ordering::Relaxed);
        check(_dup(-1) == -1 && errno() == EBADF && INVALID.load(Ordering::Relaxed) == before + 1);
        check(_dup2(1, -1) == -1 && errno() == EBADF);
        // stdout to a file and back.
        let saved = _dup(1);
        let out = _open(c(c"dup2.txt"), O_CREAT | O_TRUNC | O_WRONLY | O_BINARY, PMODE);
        check(saved >= 0 && out >= 0 && _dup2(out, 1) == 0);
        check(_write(1, b"to the file".as_ptr(), 11) == 11);
        check(_dup2(saved, 1) == 0 && _close(saved) == 0 && _close(out) == 0);
        let (buf, n) = read_file(c"dup2.txt");
        check(&buf[..n] == b"to the file");
        // Pipes: binary, end of the stream once the writer is closed.
        let mut p = [0i32; 2];
        check(_pipe(p.as_mut_ptr(), 256, O_BINARY) == 0 && p[0] >= 0 && p[1] > p[0]);
        check(GetFileType(_get_osfhandle(p[0])) == 3); // FILE_TYPE_PIPE
        check(_write(p[1], b"ping\n".as_ptr(), 5) == 5);
        let mut b = [0u8; 16];
        check(_read(p[0], b.as_mut_ptr(), 16) == 5 && &b[..5] == b"ping\n");
        check(_close(p[1]) == 0 && _read(p[0], b.as_mut_ptr(), 16) == 0);
        check(_close(p[0]) == 0);
        // Text mode: \n as \r\n in the pipe, \n again out of it.
        check(_pipe(p.as_mut_ptr(), 256, O_TEXT | O_NOINHERIT) == 0);
        check(_write(p[1], b"a\nb".as_ptr(), 3) == 3 && _close(p[1]) == 0);
        let mut t = [0u8; 16];
        let mut got = 0usize;
        loop {
            let n = _read(p[0], t.as_mut_ptr().add(got), (16 - got) as u32);
            if n <= 0 {
                break;
            }
            got += n as usize;
        }
        check(&t[..got] == b"a\nb" && _close(p[0]) == 0);
        check(_pipe(core::ptr::null_mut(), 0, 0) == -1 && errno() == EINVAL);
    }
}

/// `_spawn*`, `_exec*`, `_cwait`, `system`, `_popen`.
unsafe fn processes(me: *const u8) {
    // SAFETY: the runtime with valid arguments.
    unsafe {
        let null = core::ptr::null::<u8>();
        // argv[0] has no blank: the arguments are joined as they are, so a
        // path with one would split (Learn).
        let name = c(c"crtiotest");
        check(_spawnl(P_WAIT, me, name, c(c"--exit"), c(c"7"), null) == 7);
        let argv = [name, c(c"--args"), c(c"hi there"), c(c"x"), null];
        check(_spawnv(P_WAIT, me, argv.as_ptr()) == 0x21);
        let mut wme = [0u16; 128];
        for (i, &b) in cs(me).iter().enumerate() {
            wme[i] = u16::from(b);
        }
        let wname = ws(b"crtiotest\0");
        let (wexit, wthree) = (ws(b"--exit\0"), ws(b"3\0"));
        let wargv = [wname.as_ptr(), wexit.as_ptr(), wthree.as_ptr(), core::ptr::null()];
        check(_wspawnv(P_WAIT, wme.as_ptr(), wargv.as_ptr()) == 3);
        // Only the environment given.
        check(_putenv(c(c"POLY_PARENT_ONLY=1")) == 0);
        let envp = [c(c"POLY_SPAWN=on"), c(c"SystemRoot=C:\\Windows"), null];
        check(_spawnle(P_WAIT, me, name, c(c"--env"), null, envp.as_ptr()) == 0x23);
        // Descriptors: inherited unless _O_NOINHERIT.
        let fd = _open(c(c"inherit.txt"), O_CREAT | O_TRUNC | O_WRONLY | O_BINARY, PMODE);
        let no = _open(c(c"noinherit.txt"), O_CREAT | O_TRUNC | O_WRONLY | O_BINARY | O_NOINHERIT, PMODE);
        check(fd >= 3 && no >= 3);
        let mut fdtext = *b"00\0";
        fdtext[0] = b'0' + (fd / 10) as u8;
        fdtext[1] = b'0' + (fd % 10) as u8;
        let mut notext = *b"00\0";
        notext[0] = b'0' + (no / 10) as u8;
        notext[1] = b'0' + (no % 10) as u8;
        fflush(__acrt_iob_func(1));
        check(_spawnl(P_WAIT, me, name, c(c"--fds"), fdtext.as_ptr(), notext.as_ptr(), null) == 0x22);
        check(_close(fd) == 0 && _close(no) == 0);
        let (buf, n) = read_file(c"inherit.txt");
        check(&buf[..n] == b"inherited");
        // A child's standard output where _dup2 put descriptor 1.
        let saved = _dup(1);
        let out = _open(c(c"childout.txt"), O_CREAT | O_TRUNC | O_WRONLY | O_TEXT, PMODE);
        check(_dup2(out, 1) == 0 && _spawnl(P_WAIT, me, name, c(c"--echo"), null) == 0x25);
        check(_dup2(saved, 1) == 0 && _close(saved) == 0 && _close(out) == 0);
        let (buf, n) = read_file(c"childout.txt");
        check(&buf[..n] == b"child stdout\r\n");
        // Asynchronous, then _cwait.
        let h = _spawnl(P_NOWAIT, me, name, c(c"--exit"), c(c"9"), null);
        let mut status = 0;
        check(h > 0 && _cwait(&mut status, h, 0) == h && status == 9);
        // The p forms search PATH; the others only the name given.
        check(_chdir(c(c"C:\\")) == 0 && _putenv(c(c"PATH=C:\\Windows;C:\\Program Files\\PolyCrtIoTest")) == 0);
        check(_spawnlp(P_WAIT, c(c"crtiotest"), c(c"crtiotest"), c(c"--exit"), c(c"5"), null) == 5);
        check(_spawnl(P_WAIT, c(c"crtiotest"), c(c"crtiotest"), null) == -1 && errno() == ENOENT);
        check(_chdir(c(APP_DIR)) == 0);
        check(_spawnl(P_WAIT, c(c"C:\\nope.exe"), c(c"nope"), null) == -1 && errno() == ENOENT);
        let before = INVALID.load(Ordering::Relaxed);
        check(_spawnl(9, me, name, null) == -1 && errno() == EINVAL && INVALID.load(Ordering::Relaxed) == before + 1);
        // _execl: the child starts a grandchild in its place and ends.
        check(_spawnl(P_WAIT, me, name, c(c"--exec"), c(c"execd.txt"), null) == 0);
        let mut waited = 0;
        while read_file(c"execd.txt").1 != 7 && waited < 400 {
            Sleep(50);
            waited += 1;
        }
        let (buf, n) = read_file(c"execd.txt");
        check(&buf[..n] == b"exec ok");
        // system: no interpreter, then this program as COMSPEC.
        check(_putenv(c(c"COMSPEC=")) == 0);
        check(system(core::ptr::null()) == 0 && errno() == ENOENT);
        check(system(c(c"exit 1")) == -1 && errno() == ENOENT);
        let mut comspec = [0u8; 160];
        let prefix = b"COMSPEC=";
        comspec[..prefix.len()].copy_from_slice(prefix);
        comspec[prefix.len()..prefix.len() + cs(me).len()].copy_from_slice(cs(me));
        check(_putenv(comspec.as_ptr()) == 0);
        check(system(core::ptr::null()) != 0);
        check(system(c(c"exit 6")) == 6);
        check(_wsystem(ws(b"exit 4\0").as_ptr()) == 4);
        // _popen: the command's output, then its input.
        let f = _popen(c(c"echo popen-read"), c(c"r"));
        check(!f.is_null());
        let mut line = [0u8; 32];
        check(cs(fgets(line.as_mut_ptr(), 32, f)) == b"popen-read\n");
        check(fgets(line.as_mut_ptr(), 32, f).is_null());
        check(_pclose(f) == 0x11);
        let f = _popen(c(c"save popen.txt"), c(c"w"));
        check(!f.is_null() && fputs(c(c"to the child\n"), f) >= 0);
        check(_pclose(f) == 0x12);
        let (buf, n) = read_file(c"popen.txt");
        check(&buf[..n] == b"to the child\n");
        check(_popen(c(c"x"), c(c"rw")).is_null() && errno() == EINVAL);
        check(_pclose(__acrt_iob_func(1)) == -1);
        check(_putenv(c(c"COMSPEC=")) == 0);
    }
}

/// Wildcard `argv` in a child (`--glob`).
unsafe fn wildcards(me: *const u8) {
    // SAFETY: the runtime with valid arguments.
    unsafe {
        let null = core::ptr::null::<u8>();
        let r = _spawnl(
            P_WAIT,
            me,
            c(c"crtiotest"),
            c(c"--glob"),
            c(c"find\\*.txt"),
            c(c"\"find\\*.dat\""),
            c(c"nomatch*.zz"),
            null,
        );
        check(r == 0x24);
    }
}

/// `INPUT_RECORD` (`wincontypes.h`) of a key press or release.
fn key(down: bool, vk: u16, ch: u16, state: u32) -> [u8; 20] {
    let mut r = [0u8; 20];
    r[0] = 1; // KEY_EVENT
    r[4] = u8::from(down);
    r[8] = 1;
    r[10..12].copy_from_slice(&vk.to_le_bytes());
    r[14..16].copy_from_slice(&ch.to_le_bytes());
    r[16..20].copy_from_slice(&state.to_le_bytes());
    r
}

/// Press and release `ch` (its virtual key `vk`) in the console's input.
unsafe fn type_key(h: isize, vk: u16, ch: u16, state: u32) {
    let mut n = 0;
    // SAFETY: a console input handle and two records.
    unsafe {
        WriteConsoleInputW(h, key(true, vk, ch, state).as_ptr(), 1, &mut n);
        WriteConsoleInputW(h, key(false, vk, ch, state).as_ptr(), 1, &mut n);
    }
}

/// The cursor's column on the console's screen buffer.
unsafe fn column(out: isize) -> i16 {
    let mut info = [0u8; 22];
    // SAFETY: a CONSOLE_SCREEN_BUFFER_INFO (22 bytes).
    unsafe { GetConsoleScreenBufferInfo(out, info.as_mut_ptr()) };
    i16::from_le_bytes([info[4], info[5]])
}

/// The console functions: keys written into the input buffer, output
/// measured by the cursor's column.
unsafe fn console() {
    // SAFETY: the runtime and kernel32 with valid arguments.
    unsafe {
        let rw = 0x8000_0000 | 0x4000_0000;
        let input = CreateFileW(ws(b"CONIN$\0").as_ptr(), rw, 3, core::ptr::null(), 3, 0, 0);
        let out = CreateFileW(ws(b"CONOUT$\0").as_ptr(), rw, 3, core::ptr::null(), 3, 0, 0);
        check(input != -1 && out != -1);
        check(FlushConsoleInputBuffer(input) != 0 && _kbhit() == 0);
        type_key(input, 0x41, u16::from(b'a'), 0);
        check(_kbhit() != 0 && _getch() == i32::from(b'a'));
        // The release is dropped: nothing waits.
        check(_kbhit() == 0);
        // VK_UP (an enhanced key), then F1.
        type_key(input, 0x26, 0, 0x100);
        check(_getch() == 0xE0 && _getch() == 0x48);
        type_key(input, 0x70, 0, 0);
        check(_getch() == 0 && _getch() == 0x3B);
        check(_ungetch(i32::from(b'z')) == i32::from(b'z') && _ungetch(i32::from(b'y')) == -1);
        check(_kbhit() != 0 && _getch() == i32::from(b'z'));
        type_key(input, 0x45, 0xE9, 0);
        check(_getwch() == 0xE9);
        // _getche echoes; _putch, _cputs, _cprintf write at the cursor.
        check(SetConsoleCursorPosition(out, 0x0005_0000) != 0 && column(out) == 0);
        type_key(input, 0x51, u16::from(b'q'), 0);
        check(_getche() == i32::from(b'q') && column(out) == 1);
        check(_putch(i32::from(b'x')) == i32::from(b'x') && column(out) == 2);
        check(_putwch(u16::from(b'y')) == u16::from(b'y') && column(out) == 3);
        check(_cputs(c(c"abc")) == 0 && column(out) == 6);
        check(_cputws(ws(b"de\0").as_ptr()) == 0 && column(out) == 8);
        let args = [42u64, c(c"ok") as u64];
        check(
            __conio_common_vcprintf(OPTS, c(c"%d-%s"), core::ptr::null_mut(), args.as_ptr()) == 5 && column(out) == 13,
        );
        let wargs = [7u64];
        check(__conio_common_vcwprintf(OPTS, ws(b"<%d>\0").as_ptr(), core::ptr::null_mut(), wargs.as_ptr()) == 3);
        check(column(out) == 16);
        let before = INVALID.load(Ordering::Relaxed);
        check(_cputs(core::ptr::null()) != 0 && INVALID.load(Ordering::Relaxed) == before + 1);
        // _cgets: a cooked line, echoed.
        for &ch in b"hi" {
            type_key(input, u16::from(ch.to_ascii_uppercase()), u16::from(ch), 0);
        }
        type_key(input, 0x0D, 0x0D, 0);
        let mut buf = [0u8; 16];
        buf[0] = 10;
        let p = _cgets(buf.as_mut_ptr());
        check(p == buf.as_mut_ptr().add(2) && buf[1] == 2 && cs(p) == b"hi");
        for &ch in b"xyz" {
            type_key(input, u16::from(ch.to_ascii_uppercase()), u16::from(ch), 0);
        }
        type_key(input, 0x0D, 0x0D, 0);
        let mut small = [0u8; 8];
        let mut read = 0usize;
        check(_cgets_s(small.as_mut_ptr(), 8, &mut read) == 0 && read == 3 && cs(small.as_ptr()) == b"xyz");
        for &ch in b"w" {
            type_key(input, u16::from(ch.to_ascii_uppercase()), u16::from(ch), 0);
        }
        type_key(input, 0x0D, 0x0D, 0);
        let mut wbuf = [0u16; 8];
        wbuf[0] = 4;
        let wp = _cgetws(wbuf.as_mut_ptr());
        check(wp == wbuf.as_mut_ptr().add(2) && wbuf[1] == 1 && wcs(wp) == ws(b"w"));
    }
}

/// `msvcrt.dll`'s forms: `_findfirst` (its 64-bit `time_t`), `_cprintf`.
unsafe fn msvcrt() {
    // SAFETY: the runtime's documented functions with valid arguments.
    unsafe {
        let m = LoadLibraryW(ws(b"msvcrt.dll\0").as_ptr());
        check(!m.is_null());
        let get = |n: &core::ffi::CStr| GetProcAddress(m, n.as_ptr() as *const u8);
        type FindFirst = unsafe extern "C" fn(*const u8, *mut u8) -> isize;
        type Cprintf = unsafe extern "C" fn(*const u8, ...) -> i32;
        let findfirst: FindFirst = core::mem::transmute(get(c"_findfirst"));
        let findclose: unsafe extern "C" fn(isize) -> i32 = core::mem::transmute(get(c"_findclose"));
        let cprintf: Cprintf = core::mem::transmute(get(c"_cprintf"));
        // _finddata64i32_t: size at 32 (4 bytes), name at 36.
        let mut d = [0u8; 296];
        let h = findfirst(c(c"find\\a1.txt"), d.as_mut_ptr());
        check(h != -1 && u32::from_le_bytes(d[32..36].try_into().unwrap()) == 5 && cs(d.as_ptr().add(36)) == b"a1.txt");
        check(findclose(h) == 0);
        let out = CreateFileW(ws(b"CONOUT$\0").as_ptr(), 0xC000_0000, 3, core::ptr::null(), 3, 0, 0);
        check(SetConsoleCursorPosition(out, 0x0006_0000) != 0);
        check(cprintf(c(c"%s=%d"), c(c"n"), 12) == 4 && column(out) == 4);
    }
}
