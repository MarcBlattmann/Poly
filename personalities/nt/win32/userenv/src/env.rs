//! `CreateEnvironmentBlock` and `DestroyEnvironmentBlock` (Learn, `userenv.h`; the sheets
//! `specs/windows/userenv.dll/CreateEnvironmentBlock.yaml` and `DestroyEnvironmentBlock.yaml`).
//!
//! The block is the default environment of the token's user, `poly_kernelbase::defenv::build`
//! (the rule the kernel gives its first process), read from the registry through `kernelbase`'s
//! registry functions; with `bInherit` the caller's own environment is the start. This file makes
//! the calls: the token's user and session (`NtQueryInformationToken`, which needs
//! `TOKEN_QUERY`), the registry, the processor, and the allocation (`LocalAlloc`; the block is
//! the caller's until `DestroyEnvironmentBlock`).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use poly_kernelbase::defenv::{self, Block, Machine, Processor, Root, Source, UserInfo};

type Handle = isize;

#[cfg_attr(target_arch = "x86", link(name = "kernelbase", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernelbase", kind = "raw-dylib"))]
unsafe extern "system" {
    fn LocalAlloc(flags: u32, bytes: usize) -> *mut u8;
    fn LocalFree(mem: *mut u8) -> *mut u8;
    fn SetLastError(error: u32);
    fn RegOpenKeyExW(key: Handle, sub: *const u16, options: u32, sam: u32, out: *mut Handle) -> u32;
    fn RegQueryValueExW(
        key: Handle,
        name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> u32;
    fn RegEnumValueW(
        key: Handle,
        index: u32,
        name: *mut u16,
        name_len: *mut u32,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> u32;
    fn RegCloseKey(key: Handle) -> u32;
    fn GetEnvironmentStringsW() -> *mut u16;
    fn FreeEnvironmentStringsW(block: *mut u16) -> i32;
    fn GetSystemInfo(info: *mut u8);
}

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn NtQueryInformationToken(token: Handle, class: u32, info: *mut u8, length: u32, ret: *mut u32) -> u32;
}

/// `HKEY_LOCAL_MACHINE`, `HKEY_USERS` (`winreg.h`).
const HKEY_LOCAL_MACHINE: Handle = 0x8000_0002u32 as i32 as Handle;
const HKEY_USERS: Handle = 0x8000_0003u32 as i32 as Handle;
/// `KEY_READ`, `KEY_WOW64_64KEY` (`winnt.h`).
const KEY_READ: u32 = 0x2_0019;
const KEY_WOW64_64KEY: u32 = 0x100;
/// `ERROR_SUCCESS`, `ERROR_MORE_DATA`, `ERROR_INVALID_PARAMETER`, `ERROR_NOT_ENOUGH_MEMORY`
/// (`winerror.h`).
const ERROR_SUCCESS: u32 = 0;
const ERROR_MORE_DATA: u32 = 234;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
/// `TokenUser`, `TokenSessionId` (`winnt.h`, `TOKEN_INFORMATION_CLASS`).
const TOKEN_USER: u32 = 1;
const TOKEN_SESSION_ID: u32 = 12;
/// `LMEM_FIXED` (`winbase.h`).
const LMEM_FIXED: u32 = 0;

fn z(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// The registry through `kernelbase`'s functions (the 64-bit view, as a 64-bit process sees it).
struct Registry;

/// An open key, closed when dropped.
struct Key(Handle);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: a key this file opened.
        unsafe { RegCloseKey(self.0) };
    }
}

impl Registry {
    fn open(root: Root, path: &str) -> Option<Key> {
        let master = match root {
            Root::Machine => HKEY_LOCAL_MACHINE,
            Root::Users => HKEY_USERS,
        };
        let sub = z(path);
        let mut key: Handle = 0;
        // SAFETY: a NUL-terminated path and an out-pointer.
        let r = unsafe { RegOpenKeyExW(master, sub.as_ptr(), 0, KEY_READ | KEY_WOW64_64KEY, &mut key) };
        (r == ERROR_SUCCESS).then_some(Key(key))
    }
}

impl Source for Registry {
    fn value(&self, root: Root, path: &str, name: &str) -> Option<(u32, Vec<u8>)> {
        let key = Registry::open(root, path)?;
        let name = z(name);
        let mut data = vec![0u8; 512];
        loop {
            let (mut kind, mut size) = (0u32, data.len() as u32);
            // SAFETY: the buffer holds `size` bytes.
            let r = unsafe {
                RegQueryValueExW(key.0, name.as_ptr(), core::ptr::null_mut(), &mut kind, data.as_mut_ptr(), &mut size)
            };
            match r {
                ERROR_SUCCESS => {
                    data.truncate(size as usize);
                    return Some((kind, data));
                }
                ERROR_MORE_DATA if (size as usize) > data.len() => data.resize(size as usize, 0),
                _ => return None,
            }
        }
    }

    fn values(&self, root: Root, path: &str) -> Vec<(String, u32, Vec<u8>)> {
        let Some(key) = Registry::open(root, path) else { return Vec::new() };
        let mut out = Vec::new();
        let mut name = vec![0u16; 16_384];
        let mut data = vec![0u8; 1024];
        let mut index = 0;
        loop {
            let (mut name_len, mut kind, mut size) = (name.len() as u32, 0u32, data.len() as u32);
            // SAFETY: the buffers hold `name_len` units and `size` bytes.
            let r = unsafe {
                RegEnumValueW(
                    key.0,
                    index,
                    name.as_mut_ptr(),
                    &mut name_len,
                    core::ptr::null_mut(),
                    &mut kind,
                    data.as_mut_ptr(),
                    &mut size,
                )
            };
            match r {
                ERROR_SUCCESS => {
                    let n = String::from_utf16_lossy(&name[..name_len as usize]);
                    out.push((n, kind, data[..size as usize].to_vec()));
                    index += 1;
                }
                // The data is larger: the same index again with room for it.
                ERROR_MORE_DATA if (size as usize) > data.len() => data.resize(size as usize, 0),
                _ => return out,
            }
        }
    }
}

/// The caller's own environment (`GetEnvironmentStringsW`).
fn own_environment() -> Block {
    // SAFETY: a plain call; the block is freed below.
    let p = unsafe { GetEnvironmentStringsW() };
    if p.is_null() {
        return Block::new();
    }
    let mut n = 0;
    // SAFETY: the process's block ends with an empty string (two NULs).
    unsafe {
        while !(*p.add(n) == 0 && (n == 0 || *p.add(n - 1) == 0)) {
            n += 1;
        }
    }
    // SAFETY: `n + 1` units were read above.
    let b = Block::from_utf16(unsafe { core::slice::from_raw_parts(p, n + 1) });
    // SAFETY: the block `GetEnvironmentStringsW` returned.
    unsafe { FreeEnvironmentStringsW(p) };
    b
}

/// The processor count (`SYSTEM_INFO.dwNumberOfProcessors`, `sysinfoapi.h`: after the
/// architecture, page size, two address pointers and the processor mask) and `CPUID`'s vendor and
/// version information (Intel SDM Vol. 2A, "CPUID": leaves 0 and 1).
fn machine() -> Machine {
    let mut info = [0u8; 64];
    // SAFETY: a buffer larger than SYSTEM_INFO (48 bytes; 36 in a 32-bit process).
    unsafe { GetSystemInfo(info.as_mut_ptr()) };
    let ptr = core::mem::size_of::<usize>();
    let at = 8 + 3 * ptr;
    let processors = u32::from_le_bytes([info[at], info[at + 1], info[at + 2], info[at + 3]]);
    #[cfg(target_arch = "x86")]
    use core::arch::x86::__cpuid;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::__cpuid;
    let leaf0 = __cpuid(0);
    let mut vendor = [0u8; 12];
    for (i, r) in [leaf0.ebx, leaf0.edx, leaf0.ecx].into_iter().enumerate() {
        vendor[i * 4..i * 4 + 4].copy_from_slice(&r.to_le_bytes());
    }
    let processor = Some(Processor::from_cpuid(vendor, __cpuid(1).eax));
    Machine { processors, processor }
}

/// What the token says: its user (`None` for no token) or the Win32 error that ends the call.
fn user_of(token: Handle) -> Result<Option<UserInfo>, u32> {
    if token == 0 {
        return Ok(None);
    }
    let mut buf = [0u64; 64];
    let mut ret = 0u32;
    // SAFETY: a buffer of 512 bytes.
    let st = unsafe { NtQueryInformationToken(token, TOKEN_USER, buf.as_mut_ptr().cast(), 512, &mut ret) };
    if let Some(r) = defenv::token_query_failure(st) {
        return r.map(|()| None);
    }
    // TOKEN_USER (`winnt.h`): a SID_AND_ATTRIBUTES whose SID points into the buffer.
    let base = buf.as_ptr() as usize;
    let sid_at = if core::mem::size_of::<usize>() == 8 { buf[0] as usize } else { buf[0] as u32 as usize };
    let off = sid_at.wrapping_sub(base);
    let bytes: &[u8] = {
        // SAFETY: the buffer's own 512 bytes.
        let all = unsafe { core::slice::from_raw_parts(buf.as_ptr() as *const u8, 512) };
        all.get(off..).unwrap_or(&[])
    };
    let Some(sid) = poly_security::sid::Sid::from_bytes(bytes) else { return Ok(None) };
    let sid = poly_security::sddl::to_string(&sid);
    let mut session = 0u32;
    // SAFETY: a 4-byte buffer for a 4-byte class.
    let st =
        unsafe { NtQueryInformationToken(token, TOKEN_SESSION_ID, (&mut session as *mut u32).cast(), 4, &mut ret) };
    if st != 0 {
        session = 0;
    }
    let profile = profile_of(&sid);
    let name = profile.rsplit('\\').next().unwrap_or("").into();
    Ok(Some(UserInfo { name, sid, profile, session }))
}

/// The user's profile folder: `ProfileList\<sid>\ProfileImagePath` (expanded), else under
/// `C:\Users` by the logon variables' `USERPROFILE`.
fn profile_of(sid: &str) -> String {
    let r = Registry;
    let key = alloc::format!("{}\\{sid}", defenv::PROFILE_LIST);
    if let Some((kind, text)) = defenv::string_value(&r, Root::Machine, &key, "ProfileImagePath") {
        let mut b = Block::new();
        b.set("SystemDrive", defenv::POLY_LAYOUT.system_drive);
        b.set("SystemRoot", defenv::POLY_LAYOUT.system_root);
        return if kind == defenv::REG_EXPAND_SZ { b.expand(&text) } else { text };
    }
    let volatile = alloc::format!("{sid}\\Volatile Environment");
    defenv::string_value(&r, Root::Users, &volatile, "USERPROFILE").map_or_else(String::new, |(_, t)| t)
}

/// `CreateEnvironmentBlock(lpEnvironment, hToken, bInherit)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateEnvironmentBlock(environment: *mut *mut u16, token: Handle, inherit: i32) -> i32 {
    if environment.is_null() {
        // SAFETY: a plain call.
        unsafe { SetLastError(ERROR_INVALID_PARAMETER) };
        return 0;
    }
    // SAFETY: the caller's out-pointer.
    unsafe { environment.write(core::ptr::null_mut()) };
    let user = match user_of(token) {
        Ok(u) => u,
        Err(e) => {
            // SAFETY: a plain call.
            unsafe { SetLastError(e) };
            return 0;
        }
    };
    let base = if inherit != 0 { own_environment() } else { Block::new() };
    let block = defenv::build(&Registry, &defenv::POLY_LAYOUT, user.as_ref(), machine(), base).to_utf16();
    // SAFETY: no preconditions.
    let p = unsafe { LocalAlloc(LMEM_FIXED, block.len() * 2) } as *mut u16;
    if p.is_null() {
        // SAFETY: a plain call.
        unsafe { SetLastError(ERROR_NOT_ENOUGH_MEMORY) };
        return 0;
    }
    // SAFETY: `p` holds the block's units; the out-pointer is the caller's.
    unsafe {
        core::ptr::copy_nonoverlapping(block.as_ptr(), p, block.len());
        environment.write(p);
    }
    1
}

/// `DestroyEnvironmentBlock(lpEnvironment)`: frees a block of [`CreateEnvironmentBlock`];
/// NULL is `ERROR_INVALID_PARAMETER`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DestroyEnvironmentBlock(environment: *mut u16) -> i32 {
    if environment.is_null() {
        // SAFETY: a plain call.
        unsafe { SetLastError(ERROR_INVALID_PARAMETER) };
        return 0;
    }
    // SAFETY: a block LocalAlloc made in CreateEnvironmentBlock (the caller's contract).
    unsafe { LocalFree(environment.cast()) };
    1
}
