//! A write larger than the kernel heap gives in one allocation (4 MiB) through a byte pipe: an
//! overlapped `WriteFile` of 7 MiB on the client end of a named pipe, read back on the server end
//! in 64 KiB reads (Chrome's IPC writes several megabytes at once to its pipes; ADR 0225). As
//! Learn documents `CreateNamedPipeW`, `WriteFile` / `ReadFile` with an `OVERLAPPED` and
//! `GetOverlappedResult`: the write is pending until the reader has taken its data, then reports
//! every byte, and the bytes arrive in order.

use super::{Handle, INVALID_HANDLE_VALUE, out, out_num};

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn CreateNamedPipeW(
        name: *const u16,
        open: u32,
        mode: u32,
        max: u32,
        out_size: u32,
        in_size: u32,
        timeout: u32,
        sa: *const u8,
    ) -> Handle;
    fn CreateFileW(n: *const u16, a: u32, s: u32, sa: *const u8, d: u32, f: u32, t: Handle) -> Handle;
    fn WriteFile(h: Handle, buf: *const u8, len: u32, written: *mut u32, ov: *mut u8) -> i32;
    fn ReadFile(h: Handle, buf: *mut u8, len: u32, read: *mut u32, ov: *mut u8) -> i32;
    fn GetOverlappedResult(h: Handle, ov: *mut u8, n: *mut u32, wait: i32) -> i32;
    fn CreateEventW(sa: *const u8, manual: i32, initial: i32, name: *const u16) -> Handle;
    fn CloseHandle(h: Handle) -> i32;
    fn VirtualAlloc(addr: *mut u8, size: usize, kind: u32, protect: u32) -> *mut u8;
    fn VirtualFree(addr: *mut u8, size: usize, kind: u32) -> i32;
}

const PIPE_ACCESS_DUPLEX: u32 = 3;
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
const GENERIC_READ_WRITE: u32 = 0xC000_0000;
const OPEN_EXISTING: u32 = 3;
const ERROR_IO_PENDING: u32 = 997;
const MEM_COMMIT_RESERVE: u32 = 0x3000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_READWRITE: u32 = 4;
/// More than the kernel heap's 4 MiB, and not a multiple of a page.
const LEN: usize = 7 * 1024 * 1024 + 123;
const READ: usize = 64 * 1024;

/// `super::check`, naming the line of a failed check first.
#[track_caller]
fn check(ok: bool) {
    if !ok {
        out(b"k32test: bigpipe.rs line ");
        out_num(core::panic::Location::caller().line());
        out(b"\n");
    }
    super::check(ok);
}

/// An `OVERLAPPED` (x64: 32 bytes, the event at 24) with a new manual-reset event.
fn overlapped() -> [u8; 32] {
    let mut ov = [0u8; 32];
    // SAFETY: a plain kernel32 call.
    let e = unsafe { CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()) };
    check(e != 0);
    ov[24..32].copy_from_slice(&(e as u64).to_le_bytes());
    ov
}

fn event_of(ov: &[u8; 32]) -> Handle {
    u64::from_le_bytes(ov[24..32].try_into().expect("8 bytes")) as Handle
}

pub fn run() {
    let name: [u16; 25] = {
        let mut n = [0u16; 25];
        for (d, s) in n.iter_mut().zip(b"\\\\.\\pipe\\poly-k32-bigpipe") {
            *d = u16::from(*s);
        }
        n
    };
    // SAFETY: plain kernel32 calls on our own buffers, NUL-terminated name, handles we made.
    unsafe {
        let server = CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
            0,
            1,
            READ as u32,
            READ as u32,
            0,
            core::ptr::null(),
        );
        check(server != INVALID_HANDLE_VALUE);
        let client =
            CreateFileW(name.as_ptr(), GENERIC_READ_WRITE, 0, core::ptr::null(), OPEN_EXISTING, FILE_FLAG_OVERLAPPED, 0);
        check(client != INVALID_HANDLE_VALUE);
        let src = VirtualAlloc(core::ptr::null_mut(), LEN, MEM_COMMIT_RESERVE, PAGE_READWRITE);
        let dst = VirtualAlloc(core::ptr::null_mut(), LEN, MEM_COMMIT_RESERVE, PAGE_READWRITE);
        check(!src.is_null() && !dst.is_null());
        for i in 0..LEN {
            *src.add(i) = (i.wrapping_mul(7) ^ (i >> 13)) as u8;
        }
        // The write: more than the pipe holds, so it waits for the reader.
        let mut wov = overlapped();
        let mut n = 0u32;
        let w = WriteFile(client, src, LEN as u32, &mut n, wov.as_mut_ptr());
        check(w != 0 || GetLastError() == ERROR_IO_PENDING);
        // Read it back on the server end, 64 KiB at a time.
        let mut got = 0usize;
        let mut rov = overlapped();
        while got < LEN {
            let want = READ.min(LEN - got) as u32;
            let mut r = 0u32;
            if ReadFile(server, dst.add(got), want, &mut r, rov.as_mut_ptr()) == 0 {
                check(GetLastError() == ERROR_IO_PENDING);
            }
            check(GetOverlappedResult(server, rov.as_mut_ptr(), &mut r, 1) != 0);
            check(r > 0 && r <= want);
            if r == 0 {
                break;
            }
            got += r as usize;
        }
        check(got == LEN);
        let same = (0..LEN).all(|i| *src.add(i) == *dst.add(i));
        check(same);
        // The write reports every byte.
        let mut written = 0u32;
        check(GetOverlappedResult(client, wov.as_mut_ptr(), &mut written, 1) != 0);
        check(written as usize == LEN);
        CloseHandle(event_of(&wov));
        CloseHandle(event_of(&rov));
        CloseHandle(client);
        CloseHandle(server);
        VirtualFree(src, 0, MEM_RELEASE);
        VirtualFree(dst, 0, MEM_RELEASE);
    }
}
