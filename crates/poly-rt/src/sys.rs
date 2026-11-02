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
/// `request.handles` likewise for its pairs.
pub unsafe fn spawn(request: &SpawnRequest) -> Result<u32> {
    let args = [request as *const SpawnRequest as u64, SpawnRequest::SIZE as u64, 0, 0, 0, 0];
    // SAFETY: the caller vouches for the pointers in `request`.
    abi::decode(unsafe { syscall(nr::SPAWN, args) }).map(|p| p as u32)
}

/// Wait for child `pid` to end (with `no_hang`: `WouldBlock` if it has
/// not).
pub fn wait(pid: u32, no_hang: bool) -> Result<ExitStatus> {
    let flags = if no_hang { abi::wait::NO_HANG } else { 0 };
    call(nr::WAIT, [pid.into(), flags, 0, 0, 0, 0]).map(|s| ExitStatus::decode(s).unwrap_or(ExitStatus::Faulted(0)))
}

pub fn process_id() -> u32 {
    call(nr::PROCESS_ID, [0; 6]).unwrap_or(0) as u32
}

// ---- ABI version 2 ------------------------------------------------------------

/// A device request on `handle` (its ioctl).
///
/// # Safety
///
/// `arg` is whatever the device's request takes; a pointer must be valid
/// memory of the size and kind the request reads and writes.
pub unsafe fn control(handle: Handle, request: u32, arg: u64) -> Result<u64> {
    // SAFETY: the caller vouches for `arg`.
    abi::decode(unsafe { syscall(nr::CONTROL, [handle.into(), request.into(), arg, 0, 0, 0]) })
}

/// Wait until an entry of `entries` is ready (`timeout_ns`: `abi::FOREVER`
/// waits forever, 0 only looks); returns how many are.
pub fn poll(entries: &mut [abi::PollEntry], timeout_ns: u64) -> Result<usize> {
    let args = [entries.as_mut_ptr() as u64, entries.len() as u64, timeout_ns, 0, 0, 0];
    // SAFETY: `entries` is writable for its length; `PollEntry` is the
    // ABI's layout (`repr(C)`, 8 bytes).
    abi::decode(unsafe { syscall(nr::POLL, args) }).map(|n| n as usize)
}

pub fn map_handle(handle: Handle, offset: u64, len: u64, prot: u32, shared: bool) -> Result<u64> {
    let flags = if shared { abi::map::SHARED } else { 0 };
    call(nr::MAP_HANDLE, [handle.into(), offset, len, prot.into(), flags, 0])
}

pub fn memory_object(size: u64) -> Result<Handle> {
    call(nr::MEMORY_OBJECT, [size, 0, 0, 0, 0, 0]).map(|h| h as Handle)
}

pub fn seal(handle: Handle, seals: u32) -> Result<()> {
    call(nr::SEAL, [handle.into(), seals.into(), 0, 0, 0, 0]).map(|_| ())
}

pub fn set_nonblocking(handle: Handle, on: bool) -> Result<()> {
    call(nr::SET_NONBLOCKING, [handle.into(), on.into(), 0, 0, 0, 0]).map(|_| ())
}

pub fn handle_info(handle: Handle) -> Result<abi::FileInfo> {
    let mut b = [0u8; abi::FileInfo::SIZE];
    // SAFETY: `b` is writable for a FileInfo.
    abi::decode(unsafe { syscall(nr::HANDLE_INFO, [handle.into(), b.as_mut_ptr() as u64, 0, 0, 0, 0]) })?;
    Ok(abi::FileInfo::from_bytes(&b))
}

pub fn stat(path: &[u8]) -> Result<abi::FileInfo> {
    let mut b = [0u8; abi::FileInfo::SIZE];
    let args = [path.as_ptr() as u64, path.len() as u64, b.as_mut_ptr() as u64, 0, 0, 0];
    // SAFETY: `path` is readable, `b` writable for a FileInfo.
    abi::decode(unsafe { syscall(nr::STAT, args) })?;
    Ok(abi::FileInfo::from_bytes(&b))
}

/// A call taking one path and a value.
fn path_call(number: u64, path: &[u8], value: u64) -> Result<u64> {
    // SAFETY: `path` is readable for its length.
    abi::decode(unsafe { syscall(number, [path.as_ptr() as u64, path.len() as u64, value, 0, 0, 0]) })
}

pub fn rename(from: &[u8], to: &[u8]) -> Result<()> {
    let args = [from.as_ptr() as u64, from.len() as u64, to.as_ptr() as u64, to.len() as u64, 0, 0];
    // SAFETY: both paths are readable for their lengths.
    abi::decode(unsafe { syscall(nr::RENAME, args) }).map(|_| ())
}

pub fn remove(path: &[u8], directory: bool) -> Result<()> {
    path_call(nr::REMOVE, path, if directory { abi::remove::DIRECTORY } else { 0 }).map(|_| ())
}

pub fn make_dir(path: &[u8], mode: u16) -> Result<()> {
    path_call(nr::MAKE_DIR, path, mode.into()).map(|_| ())
}

pub fn set_mode(path: &[u8], mode: u16) -> Result<()> {
    path_call(nr::SET_MODE, path, mode.into()).map(|_| ())
}

pub fn make_fifo(path: &[u8], mode: u16) -> Result<()> {
    path_call(nr::MAKE_FIFO, path, mode.into()).map(|_| ())
}

/// The next records of directory `handle` into `buf`; 0 at the end.
pub fn read_dir(handle: Handle, buf: &mut [u8]) -> Result<usize> {
    let args = [handle.into(), buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0, 0];
    // SAFETY: `buf` is writable for its length.
    abi::decode(unsafe { syscall(nr::READ_DIR, args) }).map(|n| n as usize)
}

pub fn sync() {
    let _ = call(nr::SYNC, [0; 6]);
}

/// A call that writes two handles.
fn pair_call(number: u64, a: u64, b: u64) -> Result<[Handle; 2]> {
    let mut out = [0u32; 2];
    let ptr = out.as_mut_ptr() as u64;
    let args = if number == nr::PIPE { [ptr, 0, 0, 0, 0, 0] } else { [a, b, ptr, 0, 0, 0] };
    // SAFETY: `out` is writable for two handles.
    abi::decode(unsafe { syscall(number, args) })?;
    Ok(out)
}

/// A pipe: its read and write handles.
pub fn pipe() -> Result<[Handle; 2]> {
    pair_call(nr::PIPE, 0, 0)
}

pub fn socket(kind: u64, nonblock: bool) -> Result<Handle> {
    call(nr::SOCKET, [kind, nonblock.into(), 0, 0, 0, 0]).map(|h| h as Handle)
}

pub fn socket_pair(kind: u64, nonblock: bool) -> Result<[Handle; 2]> {
    pair_call(nr::SOCKET_PAIR, kind, nonblock.into())
}

pub fn bind(handle: Handle, name: &[u8], abstract_name: bool) -> Result<()> {
    let flags = if abstract_name { abi::socket::ABSTRACT } else { 0 };
    let args = [handle.into(), name.as_ptr() as u64, name.len() as u64, flags, 0, 0];
    // SAFETY: `name` is readable for its length.
    abi::decode(unsafe { syscall(nr::BIND, args) }).map(|_| ())
}

pub fn listen(handle: Handle, backlog: u32) -> Result<()> {
    call(nr::LISTEN, [handle.into(), backlog.into(), 0, 0, 0, 0]).map(|_| ())
}

pub fn accept(handle: Handle, nonblock: bool) -> Result<Handle> {
    call(nr::ACCEPT, [handle.into(), nonblock.into(), 0, 0, 0, 0]).map(|h| h as Handle)
}

pub fn connect(handle: Handle, name: &[u8], abstract_name: bool) -> Result<()> {
    let flags = if abstract_name { abi::socket::ABSTRACT } else { 0 };
    let args = [handle.into(), name.as_ptr() as u64, name.len() as u64, flags, 0, 0];
    // SAFETY: `name` is readable for its length.
    abi::decode(unsafe { syscall(nr::CONNECT, args) }).map(|_| ())
}

/// Send `data` and `handles` (with its first byte).
pub fn send(handle: Handle, data: &[u8], handles: &[Handle]) -> Result<usize> {
    let args =
        [handle.into(), data.as_ptr() as u64, data.len() as u64, handles.as_ptr() as u64, handles.len() as u64, 0];
    // SAFETY: `data` and `handles` are readable for their lengths.
    abi::decode(unsafe { syscall(nr::SEND, args) }).map(|n| n as usize)
}

/// Receive into `buf`, and the handles that came with the bytes into
/// `handles` (as many as it has room for); returns the bytes and the
/// handle count.
pub fn recv(handle: Handle, buf: &mut [u8], handles: &mut [Handle]) -> Result<(usize, usize)> {
    let mut count = handles.len() as u32;
    let args = [
        handle.into(),
        buf.as_mut_ptr() as u64,
        buf.len() as u64,
        handles.as_mut_ptr() as u64,
        &mut count as *mut u32 as u64,
        0,
    ];
    // SAFETY: `buf`, `handles` and `count` are writable for their lengths.
    let n = abi::decode(unsafe { syscall(nr::RECV, args) })?;
    Ok((n as usize, count as usize))
}

pub fn socket_peer(handle: Handle) -> Result<abi::PeerInfo> {
    let mut b = [0u8; abi::PeerInfo::SIZE];
    // SAFETY: `b` is writable for a PeerInfo.
    abi::decode(unsafe { syscall(nr::SOCKET_PEER, [handle.into(), b.as_mut_ptr() as u64, 0, 0, 0, 0]) })?;
    Ok(abi::PeerInfo::from_bytes(&b))
}

pub fn process_handle(pid: u32) -> Result<Handle> {
    call(nr::PROCESS_HANDLE, [pid.into(), 0, 0, 0, 0, 0]).map(|h| h as Handle)
}

pub fn signal(pid: u32, what: u64) -> Result<()> {
    call(nr::SIGNAL, [pid.into(), what, 0, 0, 0, 0]).map(|_| ())
}

/// The network interfaces' records (`abi::InterfaceRecord`) into `buf`.
pub fn net_interfaces(buf: &mut [u8]) -> Result<usize> {
    // SAFETY: `buf` is writable for its length.
    abi::decode(unsafe { syscall(nr::NET_INTERFACES, [buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0, 0, 0]) })
        .map(|n| n as usize)
}

pub fn truncate(handle: Handle, len: u64) -> Result<()> {
    call(nr::TRUNCATE, [handle.into(), len, 0, 0, 0, 0]).map(|_| ())
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
