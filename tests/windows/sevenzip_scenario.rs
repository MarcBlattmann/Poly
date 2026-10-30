//! The 7-Zip scenario driver (ADR 0088): runs the pinned `7za.exe` (7-Zip
//! 26.05, x64, ADR 0087) step by step, identically on real Windows and on
//! Poly, and records what each step did.
//!
//! It runs beside `7za.exe` in a directory of its own, which is also the
//! current directory of every step. It
//!
//! 1. records facts about the machine that 7-Zip's output can depend on
//!    (the Windows version, the ANSI and OEM code pages, the local time
//!    offset and time zone, the processor count, the memory size, whether
//!    `HKCU\Software\7-Zip` exists) and the inputs (`7za.exe`'s size and
//!    SHA-256);
//! 2. writes `hello.txt` ([`SAMPLE_TEXT`]) and sets its three times to
//!    2026-01-01 00:00:00 UTC (`SetFileTime`), so the archives are the
//!    same on every run;
//! 3. runs every step of [`STEPS`] in order with `CreateProcessW`
//!    (`bInheritHandles`, `STARTF_USESTDHANDLES`): standard input an empty
//!    file, standard output and error two files; waits for it (killing it
//!    after [`STEP_TIMEOUT_MS`]) and records its exit code, the bytes of
//!    both outputs and every file and directory below the directory but
//!    the driver's own (path, size, SHA-256, last-write time, attributes,
//!    and the bytes of files up to [`INLINE_MAX`]).
//!
//! A last-write time at or after the start of a step is written as the
//! name of the step that was running (`mtime=step:add`), which needs no
//! clock to agree between runs; an older one as the `FILETIME` value.
//! Before each step the driver waits for the system time to move past the
//! end of the previous one, so a coarse clock cannot put a file into the
//! wrong step.
//!
//! The observation file (`poly-test.out`, format `poly-scenario/1`) is
//! read by `tools/pipeline/src/scenario.rs`, which compares a Windows run
//! with Poly's. Every call here is documented on Microsoft Learn (the
//! function pages named at each declaration); the layouts are the public
//! SDK headers' (`minwinbase.h`, `processthreadsapi.h`, `fileapi.h`,
//! `sysinfoapi.h`, `timezoneapi.h`, `winnt.h`, `wdm.h`). Functions Poly
//! may lack that only feed machine facts are looked up with
//! `GetProcAddress`, so a missing one is a fact, not a load failure.
#![no_std]
#![no_main]

use core::cell::UnsafeCell;
use core::ffi::c_void;
use poly_wtest::*;

// ----- the scenario ------------------------------------------------------------------------

/// The text file the steps archive (`tools/xtask/src/sevenzip.rs`'s
/// `SAMPLE_TEXT`).
const SAMPLE_FILE: &str = "hello.txt";
const SAMPLE_TEXT: &[u8] = b"Hello from 7-Zip on Poly.\r\n";
/// 2026-01-01 00:00:00 UTC as a `FILETIME` (100 ns intervals since
/// 1601-01-01 UTC): (1767225600 + 11644473600) * 10^7.
const PINNED_TIME: u64 = 134_116_992_000_000_000;

/// One run of `7za.exe`.
struct Step {
    name: &'static str,
    /// The arguments after the program name. Steps with `common` get
    /// [`COMMON`] after their first argument (the command).
    args: &'static [&'static str],
    common: bool,
}

/// No progress display (`-bd`) and UTF-8 for the console output (`-scc`),
/// so the output does not depend on the console's code page.
const COMMON: &[&str] = &["-bd", "-sccUTF-8"];

/// The scenario, in order, in one directory (ADR 0088).
const STEPS: &[Step] = &[
    Step { name: "usage", args: &[], common: false },
    Step { name: "info", args: &["i"], common: true },
    Step { name: "hash", args: &["h", SAMPLE_FILE], common: true },
    Step { name: "add", args: &["a", "-mmt1", "new.7z", SAMPLE_FILE], common: true },
    Step { name: "list", args: &["l", "new.7z"], common: true },
    Step { name: "list-tech", args: &["l", "-slt", "new.7z"], common: true },
    Step { name: "test", args: &["t", "new.7z"], common: true },
    Step { name: "extract", args: &["x", "-y", "-oout", "new.7z"], common: true },
    Step { name: "add-mx9", args: &["a", "-mx9", "-mmt1", "all.7z", SAMPLE_FILE], common: true },
    Step { name: "add-zip", args: &["a", "-tzip", "-mmt1", "new.zip", SAMPLE_FILE], common: true },
    Step { name: "extract-zip", args: &["e", "-y", "-ozip", "new.zip"], common: true },
    Step { name: "missing-archive", args: &["x", "-y", "missing.7z"], common: true },
    Step { name: "not-an-archive", args: &["t", SAMPLE_FILE], common: true },
    Step { name: "bad-switch", args: &["a", "-zzz", "x.7z", SAMPLE_FILE], common: true },
    Step { name: "bad-command", args: &["q"], common: false },
];

/// The program the steps run, beside the driver.
const PROGRAM: &str = "7za.exe";
/// Its name on the command lines.
const ARGV0: &str = "7za";
/// The files the driver uses for each step's standard handles.
const STDIN_FILE: &str = "poly-step.in";
const STDOUT_FILE: &str = "poly-step.out";
const STDERR_FILE: &str = "poly-step.err";
/// Files of the directory that are not the scenario's (besides the
/// driver's own executable, whatever its name).
const OWN_FILES: &[&str] = &[PROGRAM, OUTPUT_FILE, STDIN_FILE, STDOUT_FILE, STDERR_FILE];
/// A step that runs longer is killed (`TerminateProcess`).
const STEP_TIMEOUT_MS: u32 = 120_000;
/// Files up to this size are recorded with their bytes; standard output
/// and error up to [`OUTPUT_MAX`].
const INLINE_MAX: u64 = 64 * 1024;
const OUTPUT_MAX: u64 = 1024 * 1024;

// ----- kernel32 and ntdll ------------------------------------------------------------------

type BOOL = i32;

/// `FILETIME` (minwinbase.h).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FileTime {
    low: u32,
    high: u32,
}

impl FileTime {
    fn of(v: u64) -> FileTime {
        FileTime { low: v as u32, high: (v >> 32) as u32 }
    }
    fn value(self) -> u64 {
        u64::from(self.high) << 32 | u64::from(self.low)
    }
}

/// `SECURITY_ATTRIBUTES` (minwinbase.h).
#[repr(C)]
struct SecurityAttributes {
    length: u32,
    security_descriptor: *mut c_void,
    inherit_handle: BOOL,
}

/// `STARTUPINFOW` (processthreadsapi.h), 104 bytes on x64.
#[repr(C)]
struct StartupInfoW {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    x_size: u32,
    y_size: u32,
    x_count_chars: u32,
    y_count_chars: u32,
    fill_attribute: u32,
    flags: u32,
    show_window: u16,
    cb_reserved2: u16,
    reserved2: *mut u8,
    std_input: HANDLE,
    std_output: HANDLE,
    std_error: HANDLE,
}

/// `PROCESS_INFORMATION` (processthreadsapi.h).
#[repr(C)]
struct ProcessInformation {
    process: HANDLE,
    thread: HANDLE,
    process_id: u32,
    thread_id: u32,
}

/// `WIN32_FIND_DATAW` (minwinbase.h), 592 bytes.
#[repr(C)]
struct FindDataW {
    attributes: u32,
    creation: FileTime,
    last_access: FileTime,
    last_write: FileTime,
    size_high: u32,
    size_low: u32,
    reserved0: u32,
    reserved1: u32,
    file_name: [u16; 260],
    alternate_file_name: [u16; 14],
}

/// `OSVERSIONINFOW` (winnt.h), 276 bytes.
#[repr(C)]
struct OsVersionInfoW {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform: u32,
    csd_version: [u16; 128],
}

// fileapi.h / winnt.h / winbase.h / processthreadsapi.h constants.
const OPEN_EXISTING: u32 = 3;
const CREATE_ALWAYS: u32 = 2;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const INVALID_FILE_ATTRIBUTES: u32 = 0xFFFF_FFFF;
const STARTF_USESTDHANDLES: u32 = 0x100;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x102;
const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
/// `HKEY_CURRENT_USER` (winreg.h) and `KEY_READ` (winnt.h).
const HKEY_CURRENT_USER: usize = 0x8000_0001;
const KEY_READ: u32 = 0x2_0019;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    /// Learn: "CreateFileW function (fileapi.h)".
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *const SecurityAttributes,
        disposition: u32,
        flags: u32,
        template: HANDLE,
    ) -> HANDLE;
    /// Learn: "ReadFile function (fileapi.h)".
    fn ReadFile(file: HANDLE, buffer: *mut u8, to_read: u32, read: *mut u32, overlapped: *mut c_void) -> BOOL;
    /// Learn: "WriteFile function (fileapi.h)".
    fn WriteFile(file: HANDLE, buffer: *const u8, to_write: u32, written: *mut u32, overlapped: *mut c_void) -> BOOL;
    /// Learn: "SetFileTime function (fileapi.h)".
    fn SetFileTime(file: HANDLE, creation: *const FileTime, access: *const FileTime, write: *const FileTime) -> BOOL;
    /// Learn: "CloseHandle function (handleapi.h)".
    fn CloseHandle(handle: HANDLE) -> BOOL;
    /// Learn: "GetFileAttributesW function (fileapi.h)".
    fn GetFileAttributesW(name: *const u16) -> u32;
    /// Learn: "FindFirstFileW function (fileapi.h)".
    fn FindFirstFileW(pattern: *const u16, data: *mut FindDataW) -> HANDLE;
    /// Learn: "FindNextFileW function (fileapi.h)".
    fn FindNextFileW(find: HANDLE, data: *mut FindDataW) -> BOOL;
    /// Learn: "FindClose function (fileapi.h)".
    fn FindClose(find: HANDLE) -> BOOL;
    /// Learn: "CreateProcessW function (processthreadsapi.h)".
    fn CreateProcessW(
        application: *const u16,
        command_line: *mut u16,
        process_attributes: *const SecurityAttributes,
        thread_attributes: *const SecurityAttributes,
        inherit_handles: BOOL,
        flags: u32,
        environment: *const c_void,
        current_directory: *const u16,
        startup_info: *const StartupInfoW,
        information: *mut ProcessInformation,
    ) -> BOOL;
    /// Learn: "WaitForSingleObject function (synchapi.h)".
    fn WaitForSingleObject(handle: HANDLE, milliseconds: u32) -> u32;
    /// Learn: "GetExitCodeProcess function (processthreadsapi.h)".
    fn GetExitCodeProcess(process: HANDLE, code: *mut u32) -> BOOL;
    /// Learn: "TerminateProcess function (processthreadsapi.h)".
    fn TerminateProcess(process: HANDLE, code: u32) -> BOOL;
    /// Learn: "GetModuleFileNameW function (libloaderapi.h)".
    fn GetModuleFileNameW(module: HANDLE, name: *mut u16, size: u32) -> u32;
    /// Learn: "GetModuleHandleW", "LoadLibraryW", "GetProcAddress" (libloaderapi.h).
    fn GetModuleHandleW(name: *const u16) -> HANDLE;
    fn LoadLibraryW(name: *const u16) -> HANDLE;
    fn GetProcAddress(module: HANDLE, name: *const u8) -> *const c_void;
    /// Learn: "GetACP", "GetOEMCP" (winnls.h).
    fn GetACP() -> u32;
    fn GetOEMCP() -> u32;
    /// Learn: "GetSystemInfo function (sysinfoapi.h)"; `SYSTEM_INFO`, 48 bytes.
    fn GetSystemInfo(info: *mut [u64; 6]);
    /// Learn: "GlobalMemoryStatusEx function (sysinfoapi.h)"; `MEMORYSTATUSEX`, 64 bytes.
    fn GlobalMemoryStatusEx(status: *mut [u64; 8]) -> BOOL;
    /// Learn: "GetSystemTimeAsFileTime function (sysinfoapi.h)".
    fn GetSystemTimeAsFileTime(time: *mut FileTime);
    /// Learn: "FileTimeToLocalFileTime function (fileapi.h)".
    fn FileTimeToLocalFileTime(utc: *const FileTime, local: *mut FileTime) -> BOOL;
    /// Learn: "GetLastError function (errhandlingapi.h)".
    fn GetLastError() -> u32;
    /// Learn: "Sleep function (synchapi.h)".
    fn Sleep(milliseconds: u32);
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    /// Learn: "RtlGetVersion function (wdm.h)".
    fn RtlGetVersion(info: *mut OsVersionInfoW) -> NTSTATUS;
}

/// `GetTimeZoneInformation` (timezoneapi.h; `TIME_ZONE_INFORMATION` is
/// 172 bytes: `Bias` at 0, `StandardBias` at 84, `DaylightBias` at 168).
type GetTimeZoneInformationFn = unsafe extern "system" fn(*mut [u8; 172]) -> u32;
/// `RegOpenKeyExW` / `RegCloseKey` (winreg.h).
type RegOpenKeyExWFn = unsafe extern "system" fn(usize, *const u16, u32, u32, *mut usize) -> i32;
type RegCloseKeyFn = unsafe extern "system" fn(usize) -> i32;

// ----- buffers (one thread; no allocator) --------------------------------------------------

/// A buffer the single thread of the driver owns.
struct Buffer<T>(UnsafeCell<T>);
// SAFETY: the driver has one thread; each buffer is used by one function
// at a time (none of them is reentrant).
unsafe impl<T> Sync for Buffer<T> {}

/// File reads: a multiple of 3, so base64 groups never straddle reads.
const CHUNK: usize = 48 * 1024;
static READ_BUFFER: Buffer<[u8; CHUNK]> = Buffer(UnsafeCell::new([0; CHUNK]));

const MAX_PATH_UNITS_HERE: usize = 520;
const MAX_ENTRIES: usize = 96;

/// A file or directory below the scenario's directory.
#[derive(Clone, Copy)]
struct Entry {
    /// The path relative to the directory, `\` between the parts.
    path: [u16; 260],
    len: usize,
    attributes: u32,
    size: u64,
    last_write: u64,
}

const NO_ENTRY: Entry = Entry { path: [0; 260], len: 0, attributes: 0, size: 0, last_write: 0 };

struct Tree {
    entries: [Entry; MAX_ENTRIES],
    count: usize,
    truncated: bool,
}

static TREE: Buffer<Tree> =
    Buffer(UnsafeCell::new(Tree { entries: [NO_ENTRY; MAX_ENTRIES], count: 0, truncated: false }));

// ----- wide strings ------------------------------------------------------------------------

/// A NUL-terminated UTF-16 string in a fixed buffer.
struct Wide {
    buf: [u16; MAX_PATH_UNITS_HERE],
    len: usize,
    overflow: bool,
}

impl Wide {
    const fn new() -> Wide {
        Wide { buf: [0; MAX_PATH_UNITS_HERE], len: 0, overflow: false }
    }

    fn units(&mut self, u: &[u16]) -> &mut Self {
        for &c in u {
            if self.len + 1 >= self.buf.len() {
                self.overflow = true;
                break;
            }
            self.buf[self.len] = c;
            self.len += 1;
        }
        self.buf[self.len] = 0;
        self
    }

    fn ascii(&mut self, s: &str) -> &mut Self {
        for b in s.bytes() {
            self.units(&[u16::from(b)]);
        }
        self
    }

    fn ptr(&self) -> *const u16 {
        self.buf.as_ptr()
    }
}

/// The driver's directory with its trailing `\`, and its own file name.
struct Place {
    dir: [u16; MAX_PATH_UNITS_HERE],
    dir_len: usize,
    name: [u16; 260],
    name_len: usize,
}

fn place() -> Option<Place> {
    let mut module = [0u16; MAX_PATH_UNITS_HERE];
    // SAFETY: the buffer holds `module.len()` units.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), module.as_mut_ptr(), module.len() as u32) } as usize;
    if n == 0 || n >= module.len() {
        return None;
    }
    let cut = module[..n].iter().rposition(|&c| c == u16::from(b'\\') || c == u16::from(b'/'))? + 1;
    let mut p = Place { dir: [0; MAX_PATH_UNITS_HERE], dir_len: cut, name: [0; 260], name_len: (n - cut).min(260) };
    p.dir[..cut].copy_from_slice(&module[..cut]);
    p.name[..p.name_len].copy_from_slice(&module[cut..cut + p.name_len]);
    Some(p)
}

impl Place {
    /// `<dir>\<rel>` (rel in UTF-16).
    fn path(&self, rel: &[u16]) -> Wide {
        let mut w = Wide::new();
        w.units(&self.dir[..self.dir_len]).units(rel);
        w
    }

    fn path_ascii(&self, rel: &str) -> Wide {
        let mut w = Wide::new();
        w.units(&self.dir[..self.dir_len]).ascii(rel);
        w
    }
}

fn eq_ignore_ascii_case(a: &[u16], b: &str) -> bool {
    let lower = |c: u16| if (u16::from(b'A')..=u16::from(b'Z')).contains(&c) { c + 32 } else { c };
    a.len() == b.len() && a.iter().zip(b.bytes()).all(|(&x, y)| lower(x) == lower(u16::from(y)))
}

// ----- output lines ------------------------------------------------------------------------

/// One line of the observation file, written in pieces.
struct Out {
    buf: [u8; 512],
    len: usize,
}

impl Out {
    fn new() -> Out {
        Out { buf: [0; 512], len: 0 }
    }

    fn bytes(&mut self, b: &[u8]) -> &mut Self {
        for &c in b {
            if self.len == self.buf.len() {
                write_output(&self.buf[..self.len]);
                self.len = 0;
            }
            self.buf[self.len] = c;
            self.len += 1;
        }
        self
    }

    fn str(&mut self, s: &str) -> &mut Self {
        self.bytes(s.as_bytes())
    }

    fn dec(&mut self, v: u64) -> &mut Self {
        let mut d = [0u8; 20];
        let mut i = d.len();
        let mut u = v;
        loop {
            i -= 1;
            d[i] = b'0' + (u % 10) as u8;
            u /= 10;
            if u == 0 {
                break;
            }
        }
        let digits = d;
        self.bytes(&digits[i..])
    }

    fn signed(&mut self, v: i64) -> &mut Self {
        if v < 0 {
            self.str("-");
        }
        self.dec(v.unsigned_abs())
    }

    fn hex(&mut self, v: u64) -> &mut Self {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let n = (16 - v.leading_zeros() as usize / 4).max(1);
        self.str("0x");
        for i in (0..n).rev() {
            let c = HEX[((v >> (4 * i)) & 0xf) as usize];
            self.bytes(&[c]);
        }
        self
    }

    fn digest(&mut self, d: &[u8; 32]) -> &mut Self {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for &b in d {
            self.bytes(&[HEX[usize::from(b >> 4)], HEX[usize::from(b & 15)]]);
        }
        self
    }

    /// UTF-16 as UTF-8 (`\` as `/`; an unpaired surrogate as U+FFFD).
    fn wide(&mut self, w: &[u16]) -> &mut Self {
        for c in char::decode_utf16(w.iter().copied()) {
            let c = match c.unwrap_or('\u{fffd}') {
                '\\' => '/',
                c => c,
            };
            let mut b = [0u8; 4];
            self.str(c.encode_utf8(&mut b));
        }
        self
    }

    fn end(&mut self) {
        self.bytes(b"\n");
        write_output(&self.buf[..self.len]);
        self.len = 0;
    }
}

// ----- files -------------------------------------------------------------------------------

fn open_read(path: &Wide) -> Option<HANDLE> {
    // SAFETY: a NUL-terminated path; no security attributes or template.
    let h = unsafe {
        CreateFileW(
            path.ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            core::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            core::ptr::null_mut(),
        )
    };
    (h != INVALID_HANDLE_VALUE).then_some(h)
}

/// Read the whole file, handing each piece to `each`; the byte count.
fn read_all(path: &Wide, mut each: impl FnMut(&[u8])) -> Option<u64> {
    let h = open_read(path)?;
    let mut total = 0u64;
    // SAFETY: the single thread owns the buffer during this call.
    let buf = unsafe { &mut *READ_BUFFER.0.get() };
    loop {
        let mut got = 0u32;
        // SAFETY: `buf` is writable for its length; synchronous read.
        let ok = unsafe { ReadFile(h, buf.as_mut_ptr(), buf.len() as u32, &mut got, core::ptr::null_mut()) };
        if ok == 0 || got == 0 {
            break;
        }
        each(&buf[..got as usize]);
        total += u64::from(got);
    }
    // SAFETY: the handle opened above, closed once.
    unsafe { CloseHandle(h) };
    Some(total)
}

/// `<label> size=<n> sha256=<hex>` and, if `inline` allows, the line
/// `<label>-b64 <base64>`; or `<label> unreadable error=<n>`.
fn record_bytes(label: &str, path: &Wide, inline: u64, prefix: &str, name: &[u16]) {
    let mut sha = Sha256::new();
    let Some(size) = read_all(path, |b| sha.update(b)) else {
        // SAFETY: no preconditions.
        let e = unsafe { GetLastError() };
        Out::new().str(label).str(" unreadable error=").dec(u64::from(e)).str(prefix).wide(name).end();
        return;
    };
    Out::new().str(label).str(" size=").dec(size).str(" sha256=").digest(&sha.finish()).str(prefix).wide(name).end();
    if size <= inline {
        Out::new().str(label).str("-b64 ").bytes(prefix.trim_start().as_bytes()).wide(name).end_without_newline();
        if !name.is_empty() {
            write_output(b" ");
        }
        let mut b64 = Base64Writer::new();
        read_all(path, |b| b64.update(b));
        b64.finish();
        write_output(b"\n");
    }
}

impl Out {
    /// Write what is buffered without ending the line.
    fn end_without_newline(&mut self) {
        write_output(&self.buf[..self.len]);
        self.len = 0;
    }
}

/// Create (or empty) a file; inheritable if `inherit`.
fn create_file(path: &Wide, access: u32, inherit: bool) -> Option<HANDLE> {
    let sa = SecurityAttributes {
        length: core::mem::size_of::<SecurityAttributes>() as u32,
        security_descriptor: core::ptr::null_mut(),
        inherit_handle: BOOL::from(inherit),
    };
    // SAFETY: a NUL-terminated path, a valid SECURITY_ATTRIBUTES.
    let h = unsafe {
        CreateFileW(
            path.ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            &sa,
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            core::ptr::null_mut(),
        )
    };
    (h != INVALID_HANDLE_VALUE).then_some(h)
}

fn now() -> u64 {
    let mut t = FileTime::default();
    // SAFETY: `t` is writable.
    unsafe { GetSystemTimeAsFileTime(&mut t) };
    t.value()
}

// ----- the tree ----------------------------------------------------------------------------

/// Every file and directory below `rel` (empty: the scenario's directory)
/// into the tree, but the driver's own files at the top.
fn walk(place: &Place, rel: &[u16], depth: usize) {
    let mut pattern = place.path(rel);
    if !rel.is_empty() {
        pattern.ascii("\\");
    }
    pattern.ascii("*");
    // SAFETY: zeroed is a valid WIN32_FIND_DATAW (integers and arrays).
    let mut data: FindDataW = unsafe { core::mem::zeroed() };
    // SAFETY: a NUL-terminated pattern; `data` is writable.
    let find = unsafe { FindFirstFileW(pattern.ptr(), &mut data) };
    if find == INVALID_HANDLE_VALUE {
        return;
    }
    loop {
        let name_len = data.file_name.iter().position(|&c| c == 0).unwrap_or(data.file_name.len());
        let name = &data.file_name[..name_len];
        let dot = name == [u16::from(b'.')] || name == [u16::from(b'.'), u16::from(b'.')];
        let own = rel.is_empty()
            && (OWN_FILES.iter().any(|f| eq_ignore_ascii_case(name, f)) || name == &place.name[..place.name_len]);
        if !dot && !own {
            // SAFETY: the single thread owns the tree during the walk.
            let tree = unsafe { &mut *TREE.0.get() };
            let mut e = NO_ENTRY;
            let mut fits = rel.len() + 1 + name.len() <= e.path.len();
            if fits {
                e.path[..rel.len()].copy_from_slice(rel);
                let mut at = rel.len();
                if !rel.is_empty() {
                    e.path[at] = u16::from(b'\\');
                    at += 1;
                }
                e.path[at..at + name.len()].copy_from_slice(name);
                e.len = at + name.len();
            }
            e.attributes = data.attributes;
            e.size = u64::from(data.size_high) << 32 | u64::from(data.size_low);
            e.last_write = data.last_write.value();
            if tree.count == MAX_ENTRIES {
                tree.truncated = true;
                fits = false;
            }
            if fits {
                tree.entries[tree.count] = e;
                tree.count += 1;
                if e.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 && depth < 8 {
                    let sub = e;
                    walk(place, &sub.path[..sub.len], depth + 1);
                }
            } else {
                tree.truncated = true;
            }
        }
        // SAFETY: `find` is the open search; `data` is writable.
        if unsafe { FindNextFileW(find, &mut data) } == 0 {
            break;
        }
    }
    // SAFETY: the search opened above, closed once.
    unsafe { FindClose(find) };
}

/// The tree, sorted by path (UTF-16 units, as an ordinal comparison).
fn scan(place: &Place) -> &'static Tree {
    {
        // SAFETY: the single thread owns the tree; the borrow ends here.
        let tree = unsafe { &mut *TREE.0.get() };
        tree.count = 0;
        tree.truncated = false;
    }
    walk(place, &[], 0);
    // SAFETY: as above; the walk has ended.
    let tree = unsafe { &mut *TREE.0.get() };
    let n = tree.count;
    for i in 1..n {
        let mut j = i;
        while j > 0 && tree.entries[j - 1].path[..tree.entries[j - 1].len] > tree.entries[j].path[..tree.entries[j].len]
        {
            tree.entries.swap(j - 1, j);
            j -= 1;
        }
    }
    tree
}

/// A last-write time as the step that was running, or the value.
fn time_label(out: &mut Out, t: u64, starts: &[u64]) {
    match starts.iter().rposition(|&s| s <= t) {
        Some(i) => out.str("step:").str(STEPS[i].name),
        None => out.dec(t),
    };
}

fn record_tree(place: &Place, starts: &[u64]) {
    let tree = scan(place);
    for e in &tree.entries[..tree.count] {
        let path = &e.path[..e.len];
        if e.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            let mut o = Out::new();
            o.str("dir mtime=");
            time_label(&mut o, e.last_write, starts);
            o.str(" attr=").hex(u64::from(e.attributes)).str(" name=").wide(path).end();
            continue;
        }
        let full = place.path(path);
        let mut sha = Sha256::new();
        let read = read_all(&full, |b| sha.update(b));
        let mut o = Out::new();
        o.str("file size=").dec(e.size);
        match read {
            Some(n) => {
                o.str(" sha256=").digest(&sha.finish());
                if n != e.size {
                    o.str(" read=").dec(n);
                }
            }
            // SAFETY: no preconditions.
            None => {
                let err = unsafe { GetLastError() };
                o.str(" unreadable=").dec(u64::from(err));
            }
        }
        o.str(" mtime=");
        time_label(&mut o, e.last_write, starts);
        o.str(" attr=").hex(u64::from(e.attributes)).str(" name=").wide(path).end();
        if read.is_some() && e.size <= INLINE_MAX {
            Out::new().str("file-b64 ").wide(path).end_without_newline();
            write_output(b" ");
            let mut b64 = Base64Writer::new();
            read_all(&full, |b| b64.update(b));
            b64.finish();
            write_output(b"\n");
        }
    }
    if tree.truncated {
        Out::new().str("tree truncated").end();
    }
}

// ----- machine facts -----------------------------------------------------------------------

fn proc_address(module: HANDLE, name: &[u8]) -> *const c_void {
    if module.is_null() {
        return core::ptr::null();
    }
    // SAFETY: `name` is NUL-terminated (callers pass b"...\0").
    unsafe { GetProcAddress(module, name.as_ptr()) }
}

fn record_machine() {
    let mut v: OsVersionInfoW =
        OsVersionInfoW { size: 0, major: 0, minor: 0, build: 0, platform: 0, csd_version: [0; 128] };
    v.size = core::mem::size_of::<OsVersionInfoW>() as u32;
    // SAFETY: `v` is a writable OSVERSIONINFOW with its size set.
    let st = unsafe { RtlGetVersion(&mut v) };
    if st == STATUS_SUCCESS {
        Out::new()
            .str("machine os-version ")
            .dec(u64::from(v.major))
            .str(".")
            .dec(u64::from(v.minor))
            .str(".")
            .dec(u64::from(v.build))
            .end();
        Out::new().str("machine os-platform ").dec(u64::from(v.platform)).end();
    } else {
        Out::new().str("machine os-version unavailable status=").hex(u64::from(st as u32)).end();
    }
    // SAFETY: no preconditions.
    let (acp, oemcp) = unsafe { (GetACP(), GetOEMCP()) };
    Out::new().str("machine ansi-code-page ").dec(u64::from(acp)).end();
    Out::new().str("machine oem-code-page ").dec(u64::from(oemcp)).end();

    // What a program that turns a UTC FILETIME into local time gets for
    // the pinned time: the offset 7-Zip's listings show.
    let mut local = FileTime::default();
    // SAFETY: both pointers are valid.
    if unsafe { FileTimeToLocalFileTime(&FileTime::of(PINNED_TIME), &mut local) } != 0 {
        let minutes = (local.value() as i64 - PINNED_TIME as i64) / 600_000_000;
        Out::new().str("machine local-offset-minutes ").signed(minutes).end();
    } else {
        // SAFETY: no preconditions.
        let e = unsafe { GetLastError() };
        Out::new().str("machine local-offset-minutes unavailable error=").dec(u64::from(e)).end();
    }
    // SAFETY: a NUL-terminated name.
    let k32 = unsafe { GetModuleHandleW(utf16z(b"kernel32.dll\0").as_ptr()) };
    let tzi = proc_address(k32, b"GetTimeZoneInformation\0");
    if tzi.is_null() {
        Out::new().str("machine time-zone unavailable").end();
    } else {
        // SAFETY: GetProcAddress gave the function of this signature.
        let f: GetTimeZoneInformationFn = unsafe { core::mem::transmute(tzi) };
        let mut info = [0u8; 172];
        // SAFETY: a writable TIME_ZONE_INFORMATION.
        let id = unsafe { f(&mut info) };
        let field = |at: usize| i32::from_le_bytes([info[at], info[at + 1], info[at + 2], info[at + 3]]);
        Out::new()
            .str("machine time-zone id=")
            .signed(i64::from(id as i32))
            .str(" bias=")
            .signed(i64::from(field(0)))
            .str(" standard-bias=")
            .signed(i64::from(field(84)))
            .str(" daylight-bias=")
            .signed(i64::from(field(168)))
            .end();
    }
    let mut si = [0u64; 6];
    // SAFETY: a writable SYSTEM_INFO.
    unsafe { GetSystemInfo(&mut si) };
    // SYSTEM_INFO.dwNumberOfProcessors: the low half of the fifth
    // quadword, offset 32 (sysinfoapi.h, x64).
    Out::new().str("machine processors ").dec(si[4] & 0xFFFF_FFFF).end();
    let mut ms = [0u64; 8];
    ms[0] = 64; // MEMORYSTATUSEX.dwLength (and dwMemoryLoad 0)
    // SAFETY: a writable MEMORYSTATUSEX with dwLength set.
    if unsafe { GlobalMemoryStatusEx(&mut ms) } != 0 {
        Out::new().str("machine memory-mib ").dec(ms[1] >> 20).end();
    } else {
        // SAFETY: no preconditions.
        let e = unsafe { GetLastError() };
        Out::new().str("machine memory-mib unavailable error=").dec(u64::from(e)).end();
    }
    // HKCU\Software\7-Zip (7za.exe imports the registry functions).
    // SAFETY: a NUL-terminated name.
    let advapi = unsafe { LoadLibraryW(utf16z(b"advapi32.dll\0").as_ptr()) };
    let (open, close) = (proc_address(advapi, b"RegOpenKeyExW\0"), proc_address(advapi, b"RegCloseKey\0"));
    if open.is_null() || close.is_null() {
        Out::new().str("machine hkcu-7zip unavailable").end();
    } else {
        // SAFETY: GetProcAddress gave the functions of these signatures.
        let (open, close): (RegOpenKeyExWFn, RegCloseKeyFn) =
            unsafe { (core::mem::transmute(open), core::mem::transmute(close)) };
        let mut key = 0usize;
        // SAFETY: a NUL-terminated subkey; `key` is writable.
        let r = unsafe { open(HKEY_CURRENT_USER, utf16z(b"Software\\7-Zip\0").as_ptr(), 0, KEY_READ, &mut key) };
        if r == 0 {
            // SAFETY: the key opened above.
            unsafe { close(key) };
            Out::new().str("machine hkcu-7zip present").end();
        } else {
            Out::new().str("machine hkcu-7zip absent error=").dec(u64::from(r as u32)).end();
        }
    }
}

/// UTF-16 of a NUL-terminated ASCII byte string, in a fixed buffer.
fn utf16z(s: &[u8]) -> [u16; 64] {
    let mut w = [0u16; 64];
    for (d, &b) in w.iter_mut().zip(s.iter().take(63)) {
        *d = u16::from(b);
    }
    w
}

// ----- the steps ---------------------------------------------------------------------------

/// Write `hello.txt` and pin its times; `false` if it could not be made.
fn make_sample(t: &mut Test, place: &Place) -> bool {
    let path = place.path_ascii(SAMPLE_FILE);
    let Some(h) = create_file(&path, GENERIC_READ | GENERIC_WRITE, false) else {
        // SAFETY: no preconditions.
        let e = unsafe { GetLastError() };
        t.check("sample_created", false, i64::from(e));
        return false;
    };
    let mut written = 0u32;
    // SAFETY: the text is readable for its length; synchronous write.
    let ok =
        unsafe { WriteFile(h, SAMPLE_TEXT.as_ptr(), SAMPLE_TEXT.len() as u32, &mut written, core::ptr::null_mut()) };
    t.check("sample_written", ok != 0 && written as usize == SAMPLE_TEXT.len(), i64::from(written));
    let pinned = FileTime::of(PINNED_TIME);
    // SAFETY: an open handle; the three times are valid FILETIMEs.
    let set = unsafe { SetFileTime(h, &pinned, &pinned, &pinned) };
    // SAFETY: no preconditions.
    let err = if set == 0 { unsafe { GetLastError() } } else { 0 };
    // SAFETY: the handle opened above, closed once.
    unsafe { CloseHandle(h) };
    if set == 0 {
        Out::new().str("input hello.txt set-file-time failed error=").dec(u64::from(err)).end();
    }
    true
}

/// The command line of a step (`7za <args>`), NUL-terminated.
fn command_line(step: &Step) -> Wide {
    let mut w = Wide::new();
    w.ascii(ARGV0);
    for (i, a) in step.args.iter().enumerate() {
        w.ascii(" ").ascii(a);
        if i == 0 && step.common {
            for c in COMMON {
                w.ascii(" ").ascii(c);
            }
        }
    }
    w
}

/// Run one step; its exit line.
fn run_step(place: &Place, step: &Step, starts: &mut [u64], index: usize) {
    let mut cmd = command_line(step);
    Out::new().str("step ").str(step.name).end();
    Out::new().str("command ").wide(&cmd.buf[..cmd.len]).end();
    let (stdin, stdout, stderr) = (
        create_file(&place.path_ascii(STDIN_FILE), GENERIC_READ, true),
        create_file(&place.path_ascii(STDOUT_FILE), GENERIC_WRITE, true),
        create_file(&place.path_ascii(STDERR_FILE), GENERIC_WRITE, true),
    );
    let (Some(stdin), Some(stdout), Some(stderr)) = (stdin, stdout, stderr) else {
        // SAFETY: no preconditions.
        let e = unsafe { GetLastError() };
        Out::new().str("exit driver-error standard-handle-files error=").dec(u64::from(e)).end();
        for h in [stdin, stdout, stderr].into_iter().flatten() {
            // SAFETY: handles opened above, closed once.
            unsafe { CloseHandle(h) };
        }
        return;
    };
    // SAFETY: zeroed is a valid STARTUPINFOW (integers and null pointers).
    let mut si: StartupInfoW = unsafe { core::mem::zeroed() };
    si.cb = core::mem::size_of::<StartupInfoW>() as u32;
    si.flags = STARTF_USESTDHANDLES;
    si.std_input = stdin;
    si.std_output = stdout;
    si.std_error = stderr;
    let mut pi = ProcessInformation {
        process: core::ptr::null_mut(),
        thread: core::ptr::null_mut(),
        process_id: 0,
        thread_id: 0,
    };
    let app = place.path_ascii(PROGRAM);
    let dir = {
        // The directory without its trailing separator.
        let mut w = Wide::new();
        w.units(&place.dir[..place.dir_len.saturating_sub(1)]);
        w
    };
    // The step starts once the clock has moved past the previous one's end.
    let floor = LAST_END.get();
    let mut spins = 0;
    while now() <= floor && spins < 1000 {
        // SAFETY: no preconditions.
        unsafe { Sleep(1) };
        spins += 1;
    }
    starts[index] = now();
    // SAFETY: valid strings (the command line writable), STARTUPINFOW and
    // PROCESS_INFORMATION; the standard handles are inheritable.
    let created = unsafe {
        CreateProcessW(
            app.ptr(),
            cmd.buf.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            1,
            0,
            core::ptr::null(),
            dir.ptr(),
            &si,
            &mut pi,
        )
    };
    // SAFETY: no preconditions.
    let create_error = if created == 0 { unsafe { GetLastError() } } else { 0 };
    for h in [stdin, stdout, stderr] {
        // SAFETY: the driver's copies of the handles, closed once.
        unsafe { CloseHandle(h) };
    }
    if created == 0 {
        Out::new().str("exit spawn-error error=").dec(u64::from(create_error)).end();
    } else {
        // SAFETY: the child's process handle.
        let waited = unsafe { WaitForSingleObject(pi.process, STEP_TIMEOUT_MS) };
        if waited == WAIT_OBJECT_0 {
            let mut code = 0u32;
            // SAFETY: the child's process handle; `code` is writable.
            if unsafe { GetExitCodeProcess(pi.process, &mut code) } != 0 {
                Out::new().str("exit ").hex(u64::from(code)).end();
            } else {
                // SAFETY: no preconditions.
                let e = unsafe { GetLastError() };
                Out::new().str("exit unknown error=").dec(u64::from(e)).end();
            }
        } else {
            // SAFETY: the child's process handle.
            unsafe {
                TerminateProcess(pi.process, 1);
                WaitForSingleObject(pi.process, 10_000);
            }
            if waited == WAIT_TIMEOUT {
                Out::new().str("exit timeout ms=").dec(u64::from(STEP_TIMEOUT_MS)).end();
            } else {
                Out::new().str("exit wait-failed result=").hex(u64::from(waited)).end();
            }
        }
        // SAFETY: the handles CreateProcessW returned, closed once.
        unsafe {
            CloseHandle(pi.thread);
            CloseHandle(pi.process);
        }
    }
    LAST_END.set(now());
    record_bytes("stdout", &place.path_ascii(STDOUT_FILE), OUTPUT_MAX, "", &[]);
    record_bytes("stderr", &place.path_ascii(STDERR_FILE), OUTPUT_MAX, "", &[]);
    record_tree(place, &starts[..=index]);
    Out::new().str("end ").str(step.name).end();
}

/// When the last step's process had ended (the system time).
struct LastEnd(UnsafeCell<u64>);
// SAFETY: one thread.
unsafe impl Sync for LastEnd {}
impl LastEnd {
    fn get(&self) -> u64 {
        // SAFETY: one thread; no reference outlives the read.
        unsafe { *self.0.get() }
    }
    fn set(&self, v: u64) {
        // SAFETY: as above.
        unsafe { *self.0.get() = v }
    }
}
static LAST_END: LastEnd = LastEnd(UnsafeCell::new(0));

poly_wtest::main!(scenario);

fn scenario(t: &mut Test) {
    Out::new().str("poly-scenario/1 sevenzip steps=").dec(STEPS.len() as u64).end();
    let Some(place) = place() else {
        t.check("module_path", false, 0);
        return;
    };
    let program = place.path_ascii(PROGRAM);
    // SAFETY: a NUL-terminated path.
    if unsafe { GetFileAttributesW(program.ptr()) } == INVALID_FILE_ATTRIBUTES {
        // Deterministic on every machine: nothing else is recorded.
        Out::new().str("error ").str(PROGRAM).str(" is not beside the driver").end();
        t.check("program_present", false, 0);
        return;
    }
    record_machine();
    record_bytes("input", &program, 0, " name=", &utf16z(b"7za.exe\0")[..7]);
    if !make_sample(t, &place) {
        return;
    }
    let mut starts = [0u64; 32];
    // Before the first step: the sample, as the steps find it.
    record_tree(&place, &[]);
    for (i, step) in STEPS.iter().enumerate() {
        run_step(&place, step, &mut starts, i);
    }
    Out::new().str("done steps=").dec(STEPS.len() as u64).end();
}
