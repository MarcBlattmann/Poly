//! The native system calls, one function each (see
//! `poly_personality_native::abi` for the convention).

use crate::Result;
use crate::abi::{self, Clock, ExitStatus, Handle, SpawnRequest, Whence, nr};

/// Make system call `number` with `args`; returns `rax`.
///
/// # Safety
///
/// The pointers among `args` must be what call `number` expects: memory
/// the kernel may read (or write) for the lengths given.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
#[inline(always)]
pub unsafe fn syscall(number: u64, args: [u64; 6]) -> u64 {
    let rax;
    // SAFETY: the native ABI: `syscall` clobbers rcx and r11 and returns in
    // rax; the kernel keeps every other register and reaches memory only
    // through the pointers the caller vouched for.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") number => rax,
            in("rdi") args[0],
            in("rsi") args[1],
            in("rdx") args[2],
            in("r10") args[3],
            in("r8") args[4],
            in("r9") args[5],
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    rax
}

/// On the host there is no Poly kernel: every call is unsupported.
///
/// # Safety
///
/// None needed; `unsafe` to match the real one.
#[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
pub unsafe fn syscall(_number: u64, _args: [u64; 6]) -> u64 {
    abi::encode(Err(abi::Error::Unsupported))
}

/// A call whose arguments hold no pointers.
fn call(number: u64, args: [u64; 6]) -> Result<u64> {
    // SAFETY: no argument is a pointer.
    abi::decode(unsafe { syscall(number, args) })
}

/// End the process with status `code & 0xff`.
pub fn exit(code: i32) -> ! {
    let _ = call(nr::EXIT, [code as u64, 0, 0, 0, 0, 0]);
    // Only reached on the host, where there is no kernel to end it.
    loop {
        core::hint::spin_loop();
    }
}

/// Write `buf` to `handle`; returns the bytes written (perhaps fewer).
pub fn write(handle: Handle, buf: &[u8]) -> Result<usize> {
    let args = [handle.into(), buf.as_ptr() as u64, buf.len() as u64, 0, 0, 0];
    // SAFETY: `buf` is readable for its length.
    abi::decode(unsafe { syscall(nr::WRITE, args) }).map(|n| n as usize)
}

/// Read into `buf` from `handle`; returns the bytes read (0 at the end).
pub fn read(handle: Handle, buf: &mut [u8]) -> Result<usize> {
    let args = [handle.into(), buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0, 0];
    // SAFETY: `buf` is writable for its length.
    abi::decode(unsafe { syscall(nr::READ, args) }).map(|n| n as usize)
}

/// Open `path` with `flags` (`abi::open`); `mode` for a file it creates.
pub fn open(path: &[u8], flags: u32, mode: u16) -> Result<Handle> {
    let args = [path.as_ptr() as u64, path.len() as u64, flags.into(), mode.into(), 0, 0];
    // SAFETY: `path` is readable for its length.
    abi::decode(unsafe { syscall(nr::OPEN, args) }).map(|h| h as Handle)
}

pub fn close(handle: Handle) -> Result<()> {
    call(nr::CLOSE, [handle.into(), 0, 0, 0, 0, 0]).map(|_| ())
}

pub fn dup(handle: Handle) -> Result<Handle> {
    call(nr::DUP, [handle.into(), 0, 0, 0, 0, 0]).map(|h| h as Handle)
}

pub fn seek(handle: Handle, offset: i64, whence: Whence) -> Result<u64> {
    call(nr::SEEK, [handle.into(), offset as u64, whence as u64, 0, 0, 0])
}

/// Map `len` bytes (rounded up to pages) of zeroed memory with `prot`
/// (`abi::prot`), at `hint` if free (0: anywhere).
pub fn map(hint: u64, len: u64, prot: u32) -> Result<u64> {
    call(nr::MAP, [hint, len, prot.into(), 0, 0, 0])
}

pub fn unmap(addr: u64, len: u64) -> Result<()> {
    call(nr::UNMAP, [addr, len, 0, 0, 0, 0]).map(|_| ())
}

pub fn protect(addr: u64, len: u64, prot: u32) -> Result<()> {
    call(nr::PROTECT, [addr, len, prot.into(), 0, 0, 0]).map(|_| ())
}

/// The time on `clock` in nanoseconds (0 on the host).
pub fn clock(clock: Clock) -> u64 {
    call(nr::CLOCK, [clock as u64, 0, 0, 0, 0, 0]).unwrap_or(0)
}

pub fn sleep(ns: u64) -> Result<()> {
    call(nr::SLEEP, [ns, 0, 0, 0, 0, 0]).map(|_| ())
}

pub fn yield_now() {
    let _ = call(nr::YIELD, [0; 6]);
}

/// Start the program `request` describes; returns its pid.
///
/// # Safety
///
/// Every pointer in `request` (the path, the argument and environment
/// arrays and their strings) must be readable for its length.
pub unsafe fn spawn(request: &SpawnRequest) -> Result<u32> {
    let args = [request as *const SpawnRequest as u64, SpawnRequest::SIZE as u64, 0, 0, 0, 0];
    // SAFETY: the caller vouches for the pointers in `request`.
    abi::decode(unsafe { syscall(nr::SPAWN, args) }).map(|p| p as u32)
}

/// Wait for child `pid` to end.
pub fn wait(pid: u32) -> Result<ExitStatus> {
    call(nr::WAIT, [pid.into(), 0, 0, 0, 0, 0]).map(|s| ExitStatus::decode(s).unwrap_or(ExitStatus::Faulted(0)))
}

pub fn process_id() -> u32 {
    call(nr::PROCESS_ID, [0; 6]).unwrap_or(0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::Error;

    #[test]
    fn calls_fail_cleanly_on_the_host() {
        assert_eq!(write(1, b"x"), Err(Error::Unsupported));
        assert_eq!(open(b"/etc/motd", abi::open::READ, 0), Err(Error::Unsupported));
        assert_eq!(map(0, 4096, abi::prot::READ), Err(Error::Unsupported));
        assert_eq!(clock(Clock::Monotonic), 0);
        assert_eq!(process_id(), 0);
    }
}
