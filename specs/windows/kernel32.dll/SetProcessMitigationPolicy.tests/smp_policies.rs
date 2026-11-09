//! SetProcessMitigationPolicy, one case per child process (a policy applies to the whole process
//! and most cannot be undone): strict handle checks (plain and permanent), dynamic code (and an
//! executable allocation afterwards), extension points, image loading, font loading, system calls,
//! child processes (and a CreateProcess afterwards), signature, DEP, ASLR, control flow guard,
//! payload restrictions, side channels, shadow stack and redirection trust: whether the set
//! succeeds, what a get reads back, the error.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winnt.h, winbase.h, sddl.h, aclapi.h, jobapi2.h,
// processthreadsapi.h, winternl.h, ntstatus.h and winerror.h (constants,
// layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type PVOID = *mut c_void;
type BOOL = i32;
/// A fixed-size text buffer for formatting lines.
struct Buf<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Buf<N> {
    const fn new() -> Self {
        Buf { b: [0; N], n: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < N {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

/// `t.line` with formatting.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<400>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const WAIT_OBJECT_0: u32 = 0;
/// winbase.h: `SECURITY_ATTRIBUTES`.
#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    descriptor: PVOID,
    inherit: BOOL,
}

/// An 8-aligned scratch buffer for token information and other results.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct Raw<const N: usize> {
    b: [u8; N],
}

impl<const N: usize> Raw<N> {
    const fn new() -> Raw<N> {
        Raw { b: [0; N] }
    }
    fn p(&mut self) -> PVOID {
        self.b.as_mut_ptr().cast()
    }
    fn u32_at(&self, off: usize) -> u32 {
        u32::from_le_bytes([self.b[off], self.b[off + 1], self.b[off + 2], self.b[off + 3]])
    }
    fn u16_at(&self, off: usize) -> u16 {
        u16::from_le_bytes([self.b[off], self.b[off + 1]])
    }
    fn u64_at(&self, off: usize) -> u64 {
        let mut a = [0u8; 8];
        a.copy_from_slice(&self.b[off..off + 8]);
        u64::from_le_bytes(a)
    }
}

/// winbase.h: `STARTUPINFOW`.
#[repr(C)]
struct STARTUPINFOW {
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
    reserved2: u16,
    reserved2_ptr: *mut u8,
    std_input: HANDLE,
    std_output: HANDLE,
    std_error: HANDLE,
}

impl STARTUPINFOW {
    fn new() -> STARTUPINFOW {
        // SAFETY: all-zero is a valid STARTUPINFOW (null pointers, zero numbers).
        let mut s: STARTUPINFOW = unsafe { core::mem::zeroed() };
        s.cb = core::mem::size_of::<STARTUPINFOW>() as u32;
        s
    }
}

/// processthreadsapi.h: `PROCESS_INFORMATION`.
#[repr(C)]
struct PROCESS_INFORMATION {
    process: HANDLE,
    thread: HANDLE,
    process_id: u32,
    thread_id: u32,
}

impl PROCESS_INFORMATION {
    fn new() -> PROCESS_INFORMATION {
        PROCESS_INFORMATION {
            process: core::ptr::null_mut(),
            thread: core::ptr::null_mut(),
            process_id: 0,
            thread_id: 0,
        }
    }
}

/// The last-error code after a failed call, as a plain number.
fn gle() -> u32 {
    // SAFETY: reads the calling thread's last error.
    unsafe { GetLastError() }
}

fn clear_error() {
    // SAFETY: sets the calling thread's last error.
    unsafe { SetLastError(0xDEAD) };
}

/// The calling process's command line (UTF-16, no NUL), from the process
/// parameters block (winternl.h: `PEB.ProcessParameters` at 0x20,
/// `RTL_USER_PROCESS_PARAMETERS.CommandLine` at 0x70).
fn command_line() -> &'static [u16] {
    let peb: *const u8;
    // SAFETY: GS points at the TEB on x64; ProcessEnvironmentBlock is at 0x60.
    unsafe { core::arch::asm!("mov {}, gs:[0x60]", out(reg) peb, options(nostack, readonly, preserves_flags)) };
    // SAFETY: the PEB and the parameters block live as long as the process.
    unsafe {
        let params = *(peb.add(0x20) as *const *const u8);
        if params.is_null() {
            return &[];
        }
        let cl = &*(params.add(0x70) as *const UnicodeString);
        if cl.buffer.is_null() {
            return &[];
        }
        core::slice::from_raw_parts(cl.buffer, usize::from(cl.length) / 2)
    }
}

/// The command line to start this very executable with `arg` appended.
fn self_command(arg: &str) -> [u16; 600] {
    let mut out = [0u16; 600];
    let img = image_path();
    let mut n = 0;
    out[n] = b'"' as u16;
    n += 1;
    for &c in img {
        if n < 590 {
            out[n] = c;
            n += 1;
        }
    }
    out[n] = b'"' as u16;
    n += 1;
    out[n] = b' ' as u16;
    n += 1;
    for b in arg.bytes() {
        if n < 598 {
            out[n] = u16::from(b);
            n += 1;
        }
    }
    out
}

/// A child's exit code after it ended (waits up to ten seconds), or -1.
fn wait_exit(process: HANDLE) -> i64 {
    let mut code = 0u32;
    // SAFETY: `process` is a process handle with SYNCHRONIZE and PROCESS_QUERY_*.
    unsafe {
        if WaitForSingleObject(process, 10_000) != WAIT_OBJECT_0 {
            return -1;
        }
        if GetExitCodeProcess(process, &mut code) == 0 {
            return -2;
        }
    }
    i64::from(code)
}

/// The number after `--child ` at the end of the command line, if there is one.
fn c2_child_mode() -> Option<u32> {
    let cl = command_line();
    let key = b"--child ";
    let mut i = 0;
    while i + key.len() <= cl.len() {
        if cl[i..i + key.len()].iter().zip(key).all(|(&c, &k)| c == u16::from(k)) {
            let mut v = 0u32;
            for &c in &cl[i + key.len()..] {
                if (u16::from(b'0')..=u16::from(b'9')).contains(&c) {
                    v = v * 10 + u32::from(c - u16::from(b'0'));
                }
            }
            return Some(v);
        }
        i += 1;
    }
    None
}

/// `SetProcessMitigationPolicy(class, {flags}, size)` then `GetProcessMitigationPolicy(self, class, size)`:
/// bit 0 set succeeded, bit 1 get succeeded, bits 2..9 the first byte read back, bits 10..25 the last
/// error of a failed set.
fn c2_set_get(class: u32, flags: u32, size: usize) -> u32 {
    let mut buf = Raw::<64>::new();
    buf.b[..4].copy_from_slice(&flags.to_le_bytes());
    clear_error();
    // SAFETY: a 64-byte buffer; `size` never exceeds 64 in the cases below.
    let ok = unsafe { SetProcessMitigationPolicy(class, buf.p(), size) };
    let err = if ok == 0 { gle() } else { 0 };
    let mut rb = Raw::<64>::new();
    // SAFETY: as above.
    let ok2 = unsafe { GetProcessMitigationPolicy(GetCurrentProcess(), class, rb.p(), size) };
    u32::from(ok != 0) | (u32::from(ok2 != 0) << 1) | ((rb.u32_at(0) & 0xFF) << 2) | ((err & 0xFFFF) << 10)
}

/// What a child does for case `n`; its return value is its exit code.
fn c2_case(n: u32) -> u32 {
    match n {
        0 => c2_set_get(3, 1, 4),
        1 => c2_set_get(3, 3, 4),
        2 => {
            let r = c2_set_get(2, 1, 4);
            // SAFETY: a plain allocation attempt of executable memory.
            let p = unsafe { VirtualAlloc(core::ptr::null_mut(), 4096, 0x3000, 0x40) };
            let err = if p.is_null() { gle() } else { 0 };
            r | (u32::from(!p.is_null()) << 26)
                | (u32::from(err != 0) << 27)
                | if err != 0 { (err & 0xF) << 28 } else { 0 }
        }
        3 => c2_set_get(6, 1, 4),
        4 => c2_set_get(10, 7, 4),
        5 => c2_set_get(9, 1, 4),
        6 => c2_set_get(4, 1, 4),
        7 => c2_set_get(13, 1, 4),
        8 => c2_set_get(8, 1, 4),
        9 => c2_set_get(0, 1, 8),
        10 => c2_set_get(1, 1, 4),
        11 => c2_set_get(100, 1, 4),
        12 => c2_set_get(3, 1, 0),
        13 => c2_set_get(3, 1, 3),
        14 => c2_set_get(3, 1, 8),
        15 => {
            clear_error();
            // SAFETY: a null buffer with size 4 is rejected by the call.
            let ok = unsafe { SetProcessMitigationPolicy(3, core::ptr::null_mut(), 4) };
            u32::from(ok != 0) | ((if ok == 0 { gle() } else { 0 } & 0xFFFF) << 10)
        }
        16 => {
            let a = c2_set_get(3, 3, 4);
            let b = c2_set_get(3, 0, 4);
            ((a & 0xF) << 0) | ((b & 0xFFFFF) << 4)
        }
        17 => c2_set_get(7, 1, 4),
        18 => c2_set_get(12, 1, 4),
        19 => c2_set_get(14, 1, 4),
        20 => c2_set_get(15, 1, 4),
        21 => c2_set_get(16, 1, 4),
        23 => c2_set_get(3, 2, 4),
        22 => {
            // a child process after ProcessChildProcessPolicy: start another copy of this program
            let r = c2_set_get(13, 1, 4);
            let mut cmd = self_command("--child 99");
            let mut si = STARTUPINFOW::new();
            let mut pi = PROCESS_INFORMATION::new();
            clear_error();
            // SAFETY: valid buffers and structures.
            let ok = unsafe {
                CreateProcessW(
                    core::ptr::null(),
                    cmd.as_mut_ptr(),
                    core::ptr::null(),
                    core::ptr::null(),
                    0,
                    0x0800_0000,
                    core::ptr::null_mut(),
                    core::ptr::null(),
                    &mut si,
                    &mut pi,
                )
            };
            let err = if ok == 0 { gle() } else { 0 };
            if ok != 0 {
                // SAFETY: handles from CreateProcessW.
                unsafe {
                    WaitForSingleObject(pi.process, 5000);
                    CloseHandle(pi.process);
                    CloseHandle(pi.thread);
                }
            }
            (r & 0x3) | (u32::from(ok != 0) << 2) | ((err & 0xFFFF) << 10)
        }
        99 => 7,
        _ => 0xFFFF_FFFF,
    }
}

/// Runs this program as `--child n` and returns its exit code (or -1).
fn c2_run_child(n: u32) -> i64 {
    let mut a = Buf::<40>::new();
    let _ = write!(a, "--child {n}");
    let mut cmd = self_command(a.as_str());
    let mut si = STARTUPINFOW::new();
    let mut pi = PROCESS_INFORMATION::new();
    // SAFETY: valid buffers and structures; CREATE_NO_WINDOW.
    let ok = unsafe {
        CreateProcessW(
            core::ptr::null(),
            cmd.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            0,
            0x0800_0000,
            core::ptr::null_mut(),
            core::ptr::null(),
            &mut si,
            &mut pi,
        )
    };
    if ok == 0 {
        return -1;
    }
    let code = wait_exit(pi.process);
    // SAFETY: closing the handles CreateProcessW returned.
    unsafe {
        CloseHandle(pi.process);
        CloseHandle(pi.thread);
    }
    code
}

/// The entry point of a test that starts itself as a child: `--child n` runs case n and exits.
macro_rules! c2_main {
    ($test:path) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn mainCRTStartup() -> ! {
            if let Some(n) = c2_child_mode() {
                exit(c2_case(n));
            }
            if open_output().is_err() {
                exit(EXIT_NO_OUTPUT);
            }
            let mut t = Test::new();
            $test(&mut t);
            let code = t.finish();
            exit(code)
        }
    };
}

/// A child's code decoded: set ok / get ok / byte read back / error.
fn c2_decode(code: i64) -> Buf<120> {
    let mut b = Buf::<120>::new();
    if code < 0 {
        let _ = write!(b, "no result ({code})");
        return b;
    }
    let c = code as u32;
    if c >= 0xC000_0000 {
        let _ = write!(b, "the child crashed, exit status {c:#x}");
        return b;
    }
    let _ = write!(
        b,
        "set_ok={} get_ok={} readback={:#x} set_error={}",
        c & 1,
        (c >> 1) & 1,
        (c >> 2) & 0xFF,
        (c >> 10) & 0xFFFF
    );
    b
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn GetExitCodeProcess(process: HANDLE, code: *mut u32) -> BOOL;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn CreateProcessW(
        app: *const u16,
        cmd: *mut u16,
        proc_attrs: *const SECURITY_ATTRIBUTES,
        thread_attrs: *const SECURITY_ATTRIBUTES,
        inherit: BOOL,
        flags: u32,
        env: PVOID,
        dir: *const u16,
        si: *mut STARTUPINFOW,
        pi: *mut PROCESS_INFORMATION,
    ) -> BOOL;
    fn GetProcessMitigationPolicy(process: HANDLE, policy: u32, buffer: PVOID, len: usize) -> BOOL;
    fn SetProcessMitigationPolicy(policy: u32, buffer: PVOID, len: usize) -> BOOL;
    fn VirtualAlloc(addr: PVOID, size: usize, kind: u32, protect: u32) -> PVOID;
}

c2_main!(test);

fn test(t: &mut Test) {
    for (n, label) in [
        (0, "StrictHandleCheck flags=1"),
        (23, "StrictHandleCheck flags=2"),
        (1, "StrictHandleCheck flags=3"),
        (2, "DynamicCode ProhibitDynamicCode=1, then VirtualAlloc(RWX)"),
        (3, "ExtensionPointDisable flags=1"),
        (4, "ImageLoad flags=7"),
        (5, "FontDisable flags=1"),
        (6, "SystemCallDisable flags=1"),
        (7, "ChildProcess flags=1"),
        (8, "Signature flags=1"),
        (9, "DEP flags=1 (8 bytes)"),
        (10, "ASLR flags=1"),
        (17, "ControlFlowGuard flags=1"),
        (18, "PayloadRestriction flags=1"),
        (19, "SideChannelIsolation flags=1"),
        (20, "UserShadowStack flags=1"),
        (21, "RedirectionTrust flags=1"),
    ] {
        let code = c2_run_child(n);
        say!(
            t,
            "{label}: {}{}",
            c2_decode(code).as_str(),
            if n == 2 { alloc_part(code) } else { Buf::<60>::new() }.as_str()
        );
    }
}

fn alloc_part(code: i64) -> Buf<60> {
    let mut b = Buf::<60>::new();
    if code >= 0 {
        let c = code as u32;
        let _ =
            write!(b, " alloc_ok={} alloc_failed={} alloc_error_low_nibble={}", (c >> 26) & 1, (c >> 27) & 1, c >> 28);
    }
    b
}
