//! The handlers of the native calls: check and decode the arguments, copy
//! user memory in and out, and call the kernel ([`NativeKernel`]).

use crate::abi::{self, Clock, Error, Handle, PAGE_SIZE, STDIO_NONE, SpawnRequest, Str, Whence, limits};
use crate::kernel::{NativeKernel, OpenHow, Prot, Spawn};
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

pub fn spawn(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    if a[1] != SpawnRequest::SIZE as u64 {
        return Err(Error::InvalidArgument);
    }
    let mut raw = [0u8; SpawnRequest::SIZE];
    k.copy_from_user(a[0], &mut raw)?;
    let req = SpawnRequest::from_bytes(&raw);
    if req.flags != 0 {
        return Err(Error::InvalidArgument);
    }
    if req.arg_count.saturating_add(req.env_count) > limits::SPAWN_STRINGS {
        return Err(Error::TooLarge);
    }
    let path = path(k, req.path.ptr, req.path.len)?;
    let mut bytes = 0;
    let args = strings(k, req.args, req.arg_count, &mut bytes)?;
    let env = strings(k, req.env, req.env_count, &mut bytes)?;
    let stdio = req.stdio.map(|h| (h != STDIO_NONE).then_some(h));
    let spawn = Spawn { path, args, env, stdio };
    k.spawn(&spawn).map(u64::from)
}

pub fn wait(k: &mut dyn NativeKernel, a: &Args) -> Result<u64, Error> {
    let pid = u32::try_from(a[0]).map_err(|_| Error::NoChild)?;
    k.wait(pid).map(abi::ExitStatus::encode)
}

pub fn process_id(k: &mut dyn NativeKernel, _: &Args) -> Result<u64, Error> {
    Ok(u64::from(k.pid()))
}
