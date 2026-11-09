//! bpf(2) (ADR 0135): the map commands, `BPF_PROG_LOAD` of trivial socket
//! filters, `BPF_OBJ_GET_INFO_BY_FD` and `BPF_MAP_FREEZE`. The maps and
//! programs are in [`crate::bpf`].
//!
//! The attribute is copied first (a NULL pointer is `EFAULT`, a size over
//! a page `E2BIG`; bytes past the union Poly knows must be zero), then
//! the command is decoded. With `kernel.unprivileged_bpf_disabled` 2,
//! creating a map and loading a program need `CAP_BPF` or `CAP_SYS_ADMIN`,
//! after the attribute is found valid.

use crate::args::Args;
use crate::bpf::{self, ATTR_SIZE, Map, MapAttr, Object, Prog, Refused};
use crate::cred::{self, cap};
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, UserPtr};
use crate::sys::io::bounce;
use crate::table::{Handler, nr};
use alloc::sync::Arc;
use alloc::vec::Vec;

/// The calls, registered in [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::BPF, bpf)];

// Commands (`enum bpf_cmd`, `linux/bpf.h`).
const MAP_CREATE: i32 = 0;
const MAP_LOOKUP_ELEM: i32 = 1;
const MAP_UPDATE_ELEM: i32 = 2;
const MAP_DELETE_ELEM: i32 = 3;
const MAP_GET_NEXT_KEY: i32 = 4;
const PROG_LOAD: i32 = 5;
const OBJ_GET_INFO_BY_FD: i32 = 15;
const MAP_FREEZE: i32 = 22;

/// The largest attribute (a page).
const SIZE_MAX: u32 = 4096;

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

/// Copy `len` bytes at `ptr`; `EFAULT` if they are not readable.
fn read_user(k: &mut dyn LinuxKernel, ptr: u64, len: usize) -> Result<Vec<u8>, Errno> {
    let mut buf = bounce(len)?;
    if len > 0 {
        k.copy_from_user(UserPtr(ptr), &mut buf)?;
    }
    Ok(buf)
}

/// Whether the bytes of the user buffer `[at, at + len)` are all zero
/// (`EFAULT` if they cannot be read).
fn zero_tail(k: &mut dyn LinuxKernel, at: u64, len: u64) -> Result<bool, Errno> {
    let mut chunk = bounce(512)?;
    let mut done = 0u64;
    while done < len {
        let n = (len - done).min(chunk.len() as u64) as usize;
        k.copy_from_user(UserPtr(at + done), &mut chunk[..n])?;
        if chunk[..n].iter().any(|&b| b != 0) {
            return Ok(false);
        }
        done += n as u64;
    }
    Ok(true)
}

/// Whether the caller may create maps and load programs.
fn bpf_capable(k: &mut dyn LinuxKernel) -> bool {
    let c = cred::current(k);
    c.capable(cap::BPF) || c.capable(cap::SYS_ADMIN)
}

/// The map behind `fd`: `EBADF` if it is not open, `EINVAL` if it is no map.
fn map_of(k: &mut dyn LinuxKernel, fd: u32) -> Result<Arc<Map>, Errno> {
    match k.bpf_object(fd as i32)? {
        Some(Object::Map(m)) => Ok(m),
        _ => Err(Errno::EINVAL),
    }
}

/// bpf(cmd, attr, size)
pub fn bpf(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (cmd, attr_ptr, size) = (a.int(0), a.ptr(1), a.uint(2));
    if attr_ptr.is_null() {
        return Err(Errno::EFAULT);
    }
    if size > SIZE_MAX {
        return Err(Errno::E2BIG);
    }
    let known = (size as usize).min(ATTR_SIZE);
    let mut attr = [0u8; ATTR_SIZE];
    if known > 0 {
        k.copy_from_user(attr_ptr, &mut attr[..known])?;
    }
    if size as usize > ATTR_SIZE
        && !zero_tail(k, attr_ptr.addr() + ATTR_SIZE as u64, u64::from(size) - ATTR_SIZE as u64)?
    {
        return Err(Errno::E2BIG);
    }
    match cmd {
        MAP_CREATE => map_create(k, &attr),
        MAP_LOOKUP_ELEM => lookup(k, &attr),
        MAP_UPDATE_ELEM => update(k, &attr),
        MAP_DELETE_ELEM => delete(k, &attr),
        MAP_GET_NEXT_KEY => next_key(k, &attr),
        PROG_LOAD => prog_load(k, &attr),
        OBJ_GET_INFO_BY_FD => get_info(k, &attr, attr_ptr),
        MAP_FREEZE => map_of(k, u32_at(&attr, 0))?.freeze().map(|()| 0),
        _ => Err(Errno::EINVAL),
    }
}

fn map_create(k: &mut dyn LinuxKernel, attr: &[u8]) -> SysResult {
    let map = Map::create(&MapAttr::parse(attr))?;
    if !bpf_capable(k) {
        return Err(Errno::EPERM);
    }
    // The descriptor is always close-on-exec.
    k.bpf_install(Object::Map(map), true).map(|fd| fd as u64)
}

/// The element commands' attribute: `map_fd`, `key`, `value`/`next_key`,
/// `flags`.
fn lookup(k: &mut dyn LinuxKernel, attr: &[u8]) -> SysResult {
    let map = map_of(k, u32_at(attr, 0))?;
    let key = read_user(k, u64_at(attr, 8), map.key_size as usize)?;
    let value = map.lookup(&key)?;
    k.copy_to_user(UserPtr(u64_at(attr, 16)), &value)?;
    Ok(0)
}

fn update(k: &mut dyn LinuxKernel, attr: &[u8]) -> SysResult {
    let map = map_of(k, u32_at(attr, 0))?;
    let key = read_user(k, u64_at(attr, 8), map.key_size as usize)?;
    let value = read_user(k, u64_at(attr, 16), map.value_size as usize)?;
    map.update(&key, &value, u64_at(attr, 24)).map(|()| 0)
}

fn delete(k: &mut dyn LinuxKernel, attr: &[u8]) -> SysResult {
    let map = map_of(k, u32_at(attr, 0))?;
    let key = read_user(k, u64_at(attr, 8), map.key_size as usize)?;
    map.delete(&key).map(|()| 0)
}

fn next_key(k: &mut dyn LinuxKernel, attr: &[u8]) -> SysResult {
    let map = map_of(k, u32_at(attr, 0))?;
    let key = match u64_at(attr, 8) {
        0 => None,
        p => Some(read_user(k, p, map.key_size as usize)?),
    };
    let next = map.next_key(key.as_deref())?;
    k.copy_to_user(UserPtr(u64_at(attr, 16)), &next)?;
    Ok(0)
}

/// BPF_PROG_LOAD: `prog_type`, `insn_cnt`, `insns`, `license`,
/// `log_level`, `log_size`, `log_buf`.
fn prog_load(k: &mut dyn LinuxKernel, attr: &[u8]) -> SysResult {
    let (prog_type, count, insns, license) =
        (u32_at(attr, 0), u32_at(attr, 4) as usize, u64_at(attr, 8), u64_at(attr, 16));
    let (log_level, log_size, log_buf) = (u32_at(attr, 24), u32_at(attr, 28) as usize, u64_at(attr, 32));
    if count > bpf::INSN_MAX {
        return Err(Errno::E2BIG);
    }
    let code = if count == 0 { Vec::new() } else { read_user(k, insns, count * 8)? };
    // The license string must be readable (at most 128 bytes).
    let mut name = [0u8; 128];
    k.strncpy_from_user(UserPtr(license), &mut name)?;
    let loaded = match Prog::load(prog_type, &code) {
        Ok(p) => p,
        Err(Refused::Error(e)) => return Err(e),
        Err(Refused::Rejected(log)) => {
            if log_level > 0 && log_size > 0 && log_buf != 0 {
                let mut text = log.into_bytes();
                text.truncate(log_size - 1);
                text.push(0);
                k.copy_to_user(UserPtr(log_buf), &text)?;
            }
            return Err(Errno::EACCES);
        }
    };
    if !bpf_capable(k) {
        return Err(Errno::EPERM);
    }
    k.bpf_install(Object::Prog(loaded), true).map(|fd| fd as u64)
}

/// BPF_OBJ_GET_INFO_BY_FD: `bpf_fd`, `info_len`, `info`. At most the
/// kernel's structure is written, `info_len` becomes what was; more
/// space than that must be zero.
fn get_info(k: &mut dyn LinuxKernel, attr: &[u8], attr_ptr: UserPtr) -> SysResult {
    let object = k.bpf_object(u32_at(attr, 0) as i32)?.ok_or(Errno::EINVAL)?;
    let (info_len, info) = (u32_at(attr, 4) as usize, u64_at(attr, 8));
    let bytes: Vec<u8> = match &object {
        Object::Map(m) => m.info().to_vec(),
        Object::Prog(p) => p.info().to_vec(),
    };
    if info_len > bytes.len() && !zero_tail(k, info + bytes.len() as u64, (info_len - bytes.len()) as u64)? {
        return Err(Errno::E2BIG);
    }
    let n = info_len.min(bytes.len());
    if n > 0 {
        k.copy_to_user(UserPtr(info), &bytes[..n])?;
    }
    k.copy_to_user(UserPtr(attr_ptr.addr() + 4), &(n as u32).to_le_bytes())?;
    Ok(0)
}
