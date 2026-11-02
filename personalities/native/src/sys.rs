//! The handlers of the native calls: check and decode the arguments, copy
//! user memory in and out, and call the kernel ([`NativeKernel`]).

use crate::abi::{
    self, Clock, DirRecord, Error, Handle, HandleMap, InterfaceRecord, PAGE_SIZE, PollEntry, STDIO_NONE, SpawnRequest,
    Str, Whence, limits,
};
use crate::kernel::{NativeKernel, OpenHow, Prot, Signal, SocketKind, SocketName, Spawn};
use alloc::vec;
use alloc::vec::Vec;

/// The six argument registers: `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9`.
pub type Args = [u64; 6];

/// A handler: the call's value, or why it failed.
pub type Handler = fn(&mut dyn NativeKernel, &Args) -> Result<u64, Error>;

/// A handle argument: handles are 32-bit.
fn handle(v: u64) -> Result<Handle, Error> {
    Handle::try_from(v).map_err(|_| Error::BadHandle)
}

/// `len` bytes of user memory at `ptr`, copied in; at most `max` bytes.
fn user_bytes(k: &dyn NativeKernel, ptr: u64, len: u64, max: u64) -> Result<Vec<u8>, Error> {
    if len > max {
        return Err(Error::TooLarge);
    }
    ptr.checked_add(len).ok_or(Error::BadAddress)?;
    let mut buf = vec![0u8; len as usize];
    if len > 0 {
        k.copy_from_user(ptr, &mut buf)?;
    }
    Ok(buf)
}

/// A path argument: not empty, no NUL, at most [`limits::PATH`] bytes.
fn path(k: &dyn NativeKernel, ptr: u64, len: u64) -> Result<Vec<u8>, Error> {
    let p = user_bytes(k, ptr, len, limits::PATH)?;
    if p.is_empty() || p.contains(&0) {
        return Err(Error::InvalidArgument);
    }
    Ok(p)
}

fn prot(bits: u64) -> Result<Prot, Error> {
    let bits = u32::try_from(bits).map_err(|_| Error::InvalidArgument)?;
    if bits & !abi::prot::ALL != 0 {
        return Err(Error::InvalidArgument);
    }
    Ok(Prot {
        read: bits & abi::prot::READ != 0,
        write: bits & abi::prot::WRITE != 0,
        exec: bits & abi::prot::EXEC != 0,
    })
}

/// `len` rounded up to whole pages; not 0.
fn page_len(len: u64) -> Result<u64, Error> {
    if len == 0 {
        return Err(Error::InvalidArgument);
    }
    len.checked_add(PAGE_SIZE - 1).map(|l| l & !(PAGE_SIZE - 1)).ok_or(Error::NoMemory)
}

/// A page-aligned range `[addr, addr + len)` with `len` rounded up.
fn page_range(addr: u64, len: u64) -> Result<(u64, u64), Error> {
    if !addr.is_multiple_of(PAGE_SIZE) {
        return Err(Error::InvalidArgument);
    }
    let len = page_len(len)?;
    addr.checked_add(len).ok_or(Error::InvalidArgument)?;
    Ok((addr, len))
}

pub fn exit(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    k.exit(a[0] as u8);
    Ok(0)
}

pub fn write(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let (h, ptr, len) = (handle(a[0])?, a[1], a[2]);
    ptr.checked_add(len).ok_or(Error::BadAddress)?;
    // In chunks, so a large write needs no large kernel buffer; a short
    // write of the kernel's ends the call with what went out.
    let mut done = 0u64;
    let mut chunk = vec![0u8; len.min(limits::TRANSFER) as usize];
    while done < len {
        let n = (len - done).min(limits::TRANSFER) as usize;
        if let Err(e) = k.copy_from_user(ptr + done, &mut chunk[..n]) {
            return if done > 0 { Ok(done) } else { Err(e) };
        }
        match k.write(h, &chunk[..n]) {
            Ok(w) => {
                done += w as u64;
                if w < n {
                    break;
                }
            }
            Err(_) if done > 0 => break,
            Err(e) => return Err(e),
        }
    }
    if len == 0 {
        // A zero-length write still checks the handle.
        k.write(h, &[])?;
    }
    Ok(done)
}

pub fn read(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let (h, ptr, len) = (handle(a[0])?, a[1], a[2]);
    ptr.checked_add(len).ok_or(Error::BadAddress)?;
    let mut buf = vec![0u8; len.min(limits::TRANSFER) as usize];
    let n = k.read(h, &mut buf)?;
    k.copy_to_user(ptr, &buf[..n])?;
    Ok(n as u64)
}

pub fn open(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let p = path(k, a[0], a[1])?;
    let flags = u32::try_from(a[2]).map_err(|_| Error::InvalidArgument)?;
    if flags & !abi::open::ALL != 0 || a[3] > 0o7777 {
        return Err(Error::InvalidArgument);
    }
    let has = |f: u32| flags & f != 0;
    let how = OpenHow {
        read: has(abi::open::READ),
        write: has(abi::open::WRITE),
        create: has(abi::open::CREATE),
        exclusive: has(abi::open::EXCLUSIVE),
        truncate: has(abi::open::TRUNCATE),
        append: has(abi::open::APPEND),
        nonblock: has(abi::open::NONBLOCK),
        directory: has(abi::open::DIRECTORY),
        mode: a[3] as u16,
    };
    // A handle reads, writes or both; truncating and appending need
    // writing, exclusive needs creating, and a directory is only read.
    let consistent = (how.read || how.write)
        && (how.write || !(how.truncate || how.append))
        && (how.create || !how.exclusive)
        && !(how.directory && (how.write || how.create));
    if !consistent {
        return Err(Error::InvalidArgument);
    }
    k.open(&p, &how).map(u64::from)
}

pub fn close(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    k.close(handle(a[0])?).map(|()| 0)
}

pub fn dup(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    k.dup(handle(a[0])?).map(u64::from)
}

pub fn seek(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let whence = Whence::from_u64(a[2]).ok_or(Error::InvalidArgument)?;
    k.seek(handle(a[0])?, a[1] as i64, whence)
}

pub fn map(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let (hint, len, p, flags) = (a[0], page_len(a[1])?, prot(a[2])?, a[3]);
    if flags != 0 || !hint.is_multiple_of(PAGE_SIZE) {
        return Err(Error::InvalidArgument);
    }
    k.map(hint, len, p)
}

pub fn unmap(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let (addr, len) = page_range(a[0], a[1])?;
    k.unmap(addr, len).map(|()| 0)
}

pub fn protect(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let (addr, len) = page_range(a[0], a[1])?;
    k.protect(addr, len, prot(a[2])?).map(|()| 0)
}

pub fn clock(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let c = Clock::from_u64(a[0]).ok_or(Error::InvalidArgument)?;
    Ok(k.clock(c))
}

pub fn sleep(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    k.sleep(a[0]).map(|()| 0)
}

pub fn yield_now(k: &mut dyn NativeKernel, _: &Args) -> Result<u64, Error> {
    k.yield_now();
    Ok(0)
}

/// The `count` strings of the [`Str`] array at `array`, copied in, adding
/// their sizes to `bytes` (at most [`limits::SPAWN_BYTES`] in all).
fn strings(k: &dyn NativeKernel, array: u64, count: u64, bytes: &mut u64) -> Result<Vec<Vec<u8>>, Error> {
    let size = count.checked_mul(Str::SIZE as u64).ok_or(Error::TooLarge)?;
    let raw = user_bytes(k, array, size, limits::SPAWN_STRINGS * Str::SIZE as u64)?;
    raw.chunks_exact(Str::SIZE)
        .map(|c| {
            let s = Str::from_bytes(c.try_into().unwrap());
            *bytes = bytes.checked_add(s.len).ok_or(Error::TooLarge)?;
            if *bytes > limits::SPAWN_BYTES {
                return Err(Error::TooLarge);
            }
            let v = user_bytes(k, s.ptr, s.len, limits::SPAWN_BYTES)?;
            if v.contains(&0) {
                return Err(Error::InvalidArgument);
            }
            Ok(v)
        })
        .collect()
}

pub fn process_id(k: &mut dyn NativeKernel, _: &Args) -> Result<u64, Error> {
    Ok(u64::from(k.pid()))
}

pub fn spawn(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    if a[1] != SpawnRequest::SIZE as u64 && a[1] != SpawnRequest::SIZE_V1 as u64 {
        return Err(Error::InvalidArgument);
    }
    let size = a[1] as usize;
    let mut raw = [0u8; SpawnRequest::SIZE];
    k.copy_from_user(a[0], &mut raw[..size])?;
    let req = SpawnRequest::from_bytes(&raw[..size]);
    if req.flags != 0 {
        return Err(Error::InvalidArgument);
    }
    if req.arg_count.saturating_add(req.env_count) > limits::SPAWN_STRINGS || req.handle_count > limits::SPAWN_HANDLES {
        return Err(Error::TooLarge);
    }
    let path = path(k, req.path.ptr, req.path.len)?;
    let mut bytes = 0;
    let args = strings(k, req.args, req.arg_count, &mut bytes)?;
    let env = strings(k, req.env, req.env_count, &mut bytes)?;
    let stdio = req.stdio.map(|h| (h != STDIO_NONE).then_some(h));
    let raw = user_bytes(
        k,
        req.handles,
        req.handle_count * HandleMap::SIZE as u64,
        limits::SPAWN_HANDLES * HandleMap::SIZE as u64,
    )?;
    let handles: Vec<(Handle, Handle)> = raw
        .chunks_exact(HandleMap::SIZE)
        .map(|c| (u32::from_le_bytes(c[0..4].try_into().unwrap()), u32::from_le_bytes(c[4..8].try_into().unwrap())))
        .collect();
    // A child handle given twice, or one of 0, 1, 2 (those are `stdio`'s).
    for (i, &(_, to)) in handles.iter().enumerate() {
        if to < 3 || handles[..i].iter().any(|&(_, t)| t == to) {
            return Err(Error::InvalidArgument);
        }
    }
    let spawn = Spawn { path, args, env, stdio, handles };
    k.spawn(&spawn).map(u64::from)
}

pub fn wait(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let pid = u32::try_from(a[0]).map_err(|_| Error::NoChild)?;
    if a[1] & !abi::wait::NO_HANG != 0 {
        return Err(Error::InvalidArgument);
    }
    k.wait(pid, a[1] & abi::wait::NO_HANG != 0).map(abi::ExitStatus::encode)
}

// ---- ABI version 2 ------------------------------------------------------------

pub fn control(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let request = u32::try_from(a[1]).map_err(|_| Error::InvalidArgument)?;
    k.control(handle(a[0])?, request, a[2])
}

pub fn poll(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let (ptr, count) = (a[0], a[1]);
    if count > limits::POLL_ENTRIES {
        return Err(Error::TooLarge);
    }
    let size = PollEntry::SIZE as u64;
    let raw = user_bytes(k, ptr, count * size, limits::POLL_ENTRIES * size)?;
    let mut entries: Vec<PollEntry> =
        raw.chunks_exact(PollEntry::SIZE).map(|c| PollEntry::from_bytes(c.try_into().unwrap())).collect();
    for e in &mut entries {
        e.revents = 0;
    }
    let timeout = (a[2] != abi::FOREVER).then_some(a[2]);
    let ready = k.poll(&mut entries, timeout)?;
    let out: Vec<u8> = entries.iter().flat_map(|e| e.to_bytes()).collect();
    if !out.is_empty() {
        k.copy_to_user(ptr, &out)?;
    }
    Ok(ready as u64)
}

pub fn map_handle(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let (h, offset, len, p, flags) = (handle(a[0])?, a[1], page_len(a[2])?, prot(a[3])?, a[4]);
    if flags & !abi::map::SHARED != 0 || !offset.is_multiple_of(PAGE_SIZE) {
        return Err(Error::InvalidArgument);
    }
    k.map_handle(h, offset, len, p, flags & abi::map::SHARED != 0)
}

pub fn memory_object(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    k.memory_object(a[0]).map(u64::from)
}

pub fn seal(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let seals = u32::try_from(a[1]).ok().filter(|s| s & !abi::seal::ALL == 0).ok_or(Error::InvalidArgument)?;
    k.seal(handle(a[0])?, seals).map(|()| 0)
}

pub fn set_nonblocking(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    k.set_nonblocking(handle(a[0])?, a[1] != 0).map(|()| 0)
}

pub fn handle_info(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let info = k.handle_info(handle(a[0])?)?;
    k.copy_to_user(a[1], &info.to_bytes()).map(|()| 0)
}

pub fn stat(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let p = path(k, a[0], a[1])?;
    let info = k.stat(&p)?;
    k.copy_to_user(a[2], &info.to_bytes()).map(|()| 0)
}

pub fn rename(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let (from, to) = (path(k, a[0], a[1])?, path(k, a[2], a[3])?);
    k.rename(&from, &to).map(|()| 0)
}

pub fn remove(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    if a[2] & !abi::remove::DIRECTORY != 0 {
        return Err(Error::InvalidArgument);
    }
    let p = path(k, a[0], a[1])?;
    k.remove(&p, a[2] & abi::remove::DIRECTORY != 0).map(|()| 0)
}

/// A mode argument: permission bits only.
fn mode(v: u64) -> Result<u16, Error> {
    if v > 0o7777 { Err(Error::InvalidArgument) } else { Ok(v as u16) }
}

pub fn make_dir(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let p = path(k, a[0], a[1])?;
    k.make_dir(&p, mode(a[2])?).map(|()| 0)
}

pub fn set_mode(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let p = path(k, a[0], a[1])?;
    k.set_mode(&p, mode(a[2])?).map(|()| 0)
}

pub fn read_dir(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let (h, ptr, len) = (handle(a[0])?, a[1], a[2].min(limits::TRANSFER));
    ptr.checked_add(len).ok_or(Error::BadAddress)?;
    let mut out = Vec::new();
    k.read_dir(h, &mut |kind, name| {
        if out.len() + DirRecord::HEADER + name.len() > len as usize {
            return false;
        }
        DirRecord { kind, name }.write(&mut out);
        true
    })?;
    if out.is_empty() {
        // Nothing fit, or the end: tell them apart by asking for one more.
        let mut more = false;
        k.read_dir(h, &mut |_, _| {
            more = true;
            false
        })?;
        if more {
            return Err(Error::TooLarge);
        }
    }
    k.copy_to_user(ptr, &out)?;
    Ok(out.len() as u64)
}

pub fn sync(k: &mut dyn NativeKernel, _: &Args) -> Result<u64, Error> {
    k.sync();
    Ok(0)
}

/// Write two handles to `at`; close them if the caller cannot learn of
/// them.
fn put_pair(k: &mut dyn NativeKernel, at: u64, pair: [Handle; 2]) -> Result<u64, Error> {
    let mut b = [0u8; 8];
    b[..4].copy_from_slice(&pair[0].to_le_bytes());
    b[4..].copy_from_slice(&pair[1].to_le_bytes());
    if let Err(e) = k.copy_to_user(at, &b) {
        let _ = k.close(pair[0]);
        let _ = k.close(pair[1]);
        return Err(e);
    }
    Ok(0)
}

pub fn pipe(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let pair = k.pipe()?;
    put_pair(k, a[0], pair)
}

pub fn make_fifo(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let p = path(k, a[0], a[1])?;
    k.make_fifo(&p, mode(a[2])?).map(|()| 0)
}

fn socket_kind(v: u64) -> Result<SocketKind, Error> {
    match v {
        abi::socket::STREAM => Ok(SocketKind::Stream),
        abi::socket::DATAGRAM => Ok(SocketKind::Datagram),
        abi::socket::SEQPACKET => Ok(SocketKind::SeqPacket),
        _ => Err(Error::InvalidArgument),
    }
}

/// `socket`'s and `accept`'s flags: only `NONBLOCK`.
fn nonblock(flags: u64) -> Result<bool, Error> {
    if flags & !abi::socket::NONBLOCK != 0 { Err(Error::InvalidArgument) } else { Ok(flags != 0) }
}

pub fn socket(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    k.socket(socket_kind(a[0])?, nonblock(a[1])?).map(u64::from)
}

pub fn socket_pair(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let pair = k.socket_pair(socket_kind(a[0])?, nonblock(a[1])?)?;
    put_pair(k, a[2], pair)
}

/// A socket name argument: a path (as `path`), or an abstract name of
/// 1 to 107 bytes (unix(7)'s `sun_path` without its leading NUL).
fn socket_name(k: &dyn NativeKernel, ptr: u64, len: u64, flags: u64) -> Result<SocketName, Error> {
    match flags {
        0 => path(k, ptr, len).map(SocketName::Path),
        abi::socket::ABSTRACT => {
            let n = user_bytes(k, ptr, len, 107)?;
            if n.is_empty() { Err(Error::InvalidArgument) } else { Ok(SocketName::Abstract(n)) }
        }
        _ => Err(Error::InvalidArgument),
    }
}

pub fn bind(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let name = socket_name(k, a[1], a[2], a[3])?;
    k.bind(handle(a[0])?, &name).map(|()| 0)
}

pub fn listen(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    k.listen(handle(a[0])?, a[1].min(4096) as u32).map(|()| 0)
}

pub fn accept(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    k.accept(handle(a[0])?, nonblock(a[1])?).map(u64::from)
}

pub fn connect(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let name = socket_name(k, a[1], a[2], a[3])?;
    k.connect(handle(a[0])?, &name).map(|()| 0)
}

/// `count` handles at `ptr`.
fn handle_list(k: &dyn NativeKernel, ptr: u64, count: u64) -> Result<Vec<Handle>, Error> {
    if count > abi::socket::MAX_HANDLES {
        return Err(Error::TooLarge);
    }
    let raw = user_bytes(k, ptr, count * 4, abi::socket::MAX_HANDLES * 4)?;
    Ok(raw.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect())
}

pub fn send(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let (h, ptr, len) = (handle(a[0])?, a[1], a[2].min(limits::TRANSFER));
    let data = user_bytes(k, ptr, len, limits::TRANSFER)?;
    let handles = handle_list(k, a[3], a[4])?;
    k.send(h, &data, &handles).map(|n| n as u64)
}

pub fn recv(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let (h, ptr, len, handles_at, count_at) = (handle(a[0])?, a[1], a[2].min(limits::TRANSFER), a[3], a[4]);
    ptr.checked_add(len).ok_or(Error::BadAddress)?;
    let mut cap = [0u8; 4];
    if count_at != 0 {
        k.copy_from_user(count_at, &mut cap)?;
    }
    let max = u32::from_le_bytes(cap).min(abi::socket::MAX_HANDLES as u32) as usize;
    let mut buf = vec![0u8; len as usize];
    let (n, got) = k.recv(h, &mut buf, max)?;
    let bytes: Vec<u8> = got.iter().flat_map(|h| h.to_le_bytes()).collect();
    let mut copied = k.copy_to_user(ptr, &buf[..n]);
    if copied.is_ok() && !bytes.is_empty() {
        copied = k.copy_to_user(handles_at, &bytes);
    }
    if copied.is_ok() && count_at != 0 {
        copied = k.copy_to_user(count_at, &(got.len() as u32).to_le_bytes());
    }
    if let Err(e) = copied {
        // Handles the caller cannot learn of would leak.
        for h in got {
            let _ = k.close(h);
        }
        return Err(e);
    }
    Ok(n as u64)
}

pub fn socket_peer(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let peer = k.socket_peer(handle(a[0])?)?;
    k.copy_to_user(a[1], &peer.to_bytes()).map(|()| 0)
}

pub fn process_handle(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let pid = u32::try_from(a[0]).map_err(|_| Error::NoChild)?;
    k.process_handle(pid).map(u64::from)
}

pub fn signal(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let pid = u32::try_from(a[0]).ok().filter(|&p| p != 0).ok_or(Error::NotFound)?;
    let what = match a[1] {
        abi::signal::PROBE => Signal::Probe,
        abi::signal::TERMINATE => Signal::Terminate,
        abi::signal::KILL => Signal::Kill,
        _ => return Err(Error::InvalidArgument),
    };
    k.signal(pid, what).map(|()| 0)
}

pub fn net_interfaces(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let mut out = Vec::new();
    for (flags, name) in k.net_interfaces() {
        InterfaceRecord { flags, name: &name }.write(&mut out);
    }
    if out.len() as u64 > a[1] {
        return Err(Error::TooLarge);
    }
    k.copy_to_user(a[0], &out)?;
    Ok(out.len() as u64)
}

pub fn truncate(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    k.truncate(handle(a[0])?, a[1]).map(|()| 0)
}
