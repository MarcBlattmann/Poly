//! Hive files of programs (ADR 0227): `NtSaveKey` writes a key and its
//! subtree to a file the caller opened, `NtLoadKey` mounts such a file as a
//! new key directly below `\REGISTRY\MACHINE` or `\REGISTRY\USER`, and
//! `NtUnloadKey` writes a loaded hive back to its file and takes it out of
//! the tree. `advapi32`'s `RegSaveKeyW`, `RegLoadKeyW` and `RegUnLoadKeyW`
//! are these (the spec sheets of the same names).
//!
//! The file format is `poly_registry::hive`'s `regf` container. A loaded
//! hive's file stays open in the kernel's handle table without sharing until
//! the unload (`RegLoadKeyW.yaml` `twice`: loading it under another name is
//! `STATUS_SHARING_VIOLATION`); its changes are written to it at the unload
//! and by `NtFlushKey` (`RegUnLoadKeyW.yaml` `persist`), never to the
//! kernel's hive store. The `<file>.LOG1` and `<file>.LOG2` files Windows keeps
//! beside a loaded hive are made empty (`RegLoadKeyW.yaml` `writes`); Poly
//! writes the hive whole and needs no log.
//!
//! Privileges (`RegLoadKeyW.yaml` `privilege`, `RegSaveKeyW.yaml`
//! `privilege`): saving needs `SeBackupPrivilege` enabled, loading and
//! unloading `SeRestorePrivilege`; without it the call is
//! `STATUS_PRIVILEGE_NOT_HELD` before anything else is looked at.
//!
//! File I/O is done outside the NT lock, on the VFS file the file object
//! holds.

use crate::abi::{self, Attributes};
use crate::io::consts::*;
use crate::io::services::{check_create, file_handle, open_in};
use crate::io::{NodeKey, Target, status_of};
use crate::kernel::NtKernel;
use crate::system::NtSystem;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_abi::SyscallRegs;
use poly_ntobj::status::*;
use poly_ntobj::{Caller, Handle, NtStatus, OBJ_CASE_INSENSITIVE, OBJ_KERNEL_HANDLE};
use poly_registry::consts::STATUS_REGISTRY_IO_FAILED;
use poly_registry::hive::{self, HiveFile, KeyImage};
use poly_registry::{HiveId, KeyId, Resolved};
use poly_vfs::OpenFile;

pub type Result<T> = core::result::Result<T, NtStatus>;

/// `STATUS_PRIVILEGE_NOT_HELD` (`ntstatus.h`).
pub const STATUS_PRIVILEGE_NOT_HELD: NtStatus = NtStatus(0xC000_0061);
/// `SE_BACKUP_PRIVILEGE` and `SE_RESTORE_PRIVILEGE` (`winnt.h`: the low
/// parts of the privileges' LUIDs).
pub const SE_BACKUP_PRIVILEGE: u32 = 17;
pub const SE_RESTORE_PRIVILEGE: u32 = 18;

/// The suffixes of the log files beside a loaded hive (`RegLoadKeyW.yaml`
/// `writes`).
pub const LOG_SUFFIXES: [&str; 2] = [".LOG1", ".LOG2"];

/// A hive loaded from a program's file.
#[derive(Debug)]
pub struct External {
    pub hive: HiveId,
    /// The kernel handle that keeps the file open (no sharing).
    pub handle: Handle,
    /// The file it was loaded from.
    pub node: NodeKey,
    pub open: Arc<OpenFile>,
}

/// The kernel-mode caller acting for `caller` (its handles are in the
/// kernel's table).
fn kernel_caller(caller: &Caller) -> Caller {
    Caller::kernel(caller.process, caller.thread)
}

fn require_privilege(sys: &mut NtSystem, caller: &Caller, luid: u32) -> Result<()> {
    if crate::security::privilege_enabled(sys, caller, luid)? { Ok(()) } else { Err(STATUS_PRIVILEGE_NOT_HELD) }
}

/// Upcase an ASCII letter (the names compared here are `\REGISTRY`).
const fn upper(c: u16) -> u16 {
    if c >= b'a' as u16 && c <= b'z' as u16 { c - 32 } else { c }
}

/// The key name of a load or unload: below the root key handle, or a full
/// `\REGISTRY\...` path (anything else is `STATUS_OBJECT_PATH_SYNTAX_BAD`, as
/// for an open).
fn resolve_target(sys: &mut NtSystem, caller: &Caller, attrs: &Attributes) -> Result<Resolved> {
    let name = attrs.name.as_deref().unwrap_or(&[]);
    let (start, path) = match attrs.root {
        Some(h) => (super::key_of_handle(sys, caller, h, 0)?.2, name),
        None => {
            let prefix: Vec<u16> = "\\REGISTRY".encode_utf16().collect();
            let ok = name.len() >= prefix.len()
                && name[..prefix.len()].iter().zip(&prefix).all(|(&a, &b)| upper(a) == b)
                && name.get(prefix.len()).is_none_or(|&c| c == u16::from(b'\\'));
            if !ok {
                return Err(STATUS_OBJECT_PATH_SYNTAX_BAD);
            }
            (sys.registry.reg.root(), &name[prefix.len()..])
        }
    };
    sys.registry.reg.resolve(start, path, true)
}

/// The file a disk file object is open on, and its VFS file.
fn disk_file(sys: &NtSystem, cookie: u64) -> Result<(NodeKey, Arc<OpenFile>)> {
    let rec = sys.io.file(cookie).ok_or(STATUS_INVALID_HANDLE)?;
    match &rec.target {
        Target::Disk(d) => {
            let open = d.open.clone().filter(|o| !o.is_path_only()).ok_or(STATUS_INVALID_DEVICE_REQUEST)?;
            Ok((d.key, open))
        }
        _ => Err(STATUS_INVALID_DEVICE_REQUEST),
    }
}

/// Open `attrs` (a file name) as the kernel for `caller`: a kernel handle
/// and the file object's cookie.
fn open_kernel(
    sys: &mut NtSystem,
    b: &mut dyn crate::Backend,
    caller: &Caller,
    attrs: &Attributes,
    desired: u32,
    share: u32,
    disposition: u32,
) -> Result<(Handle, u64)> {
    let mut a = attrs.clone();
    a.attributes |= OBJ_KERNEL_HANDLE | OBJ_CASE_INSENSITIVE;
    let options = FILE_SYNCHRONOUS_IO_NONALERT | FILE_NON_DIRECTORY_FILE;
    let params = check_create(desired | SYNCHRONIZE, share, disposition, options, FILE_ATTRIBUTE_NORMAL)?;
    let (h, _, cookie) = open_in(sys, b, &kernel_caller(caller), &a, desired | SYNCHRONIZE, params)?;
    Ok((h, cookie))
}

fn close_kernel(sys: &mut NtSystem, b: &mut dyn crate::Backend, caller: &Caller, h: Handle) {
    let _ = sys.objects.close(&kernel_caller(caller), h);
    sys.reap(b);
}

/// Read a whole file.
fn read_whole(open: &OpenFile) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut chunk = alloc::vec![0u8; 64 * 1024];
    loop {
        match open.read_at(out.len() as u64, &mut chunk).map_err(status_of)? {
            0 => return Ok(out),
            n => out.extend_from_slice(&chunk[..n]),
        }
    }
}

/// Replace a file's contents with `data`.
fn write_whole(open: &OpenFile, data: &[u8]) -> Result<()> {
    let mut done = 0;
    while done < data.len() {
        match open.write_at(done as u64, &data[done..]).map_err(status_of)? {
            0 => return Err(STATUS_REGISTRY_IO_FAILED),
            n => done += n,
        }
    }
    open.truncate(data.len() as u64).map_err(status_of)?;
    let _ = open.sync();
    Ok(())
}

/// `NtSaveKey(KeyHandle, FileHandle)`: the key, its values and its
/// non-volatile subkeys, as a hive whose root is the key (`RegSaveKeyW.yaml`).
/// The key handle needs no particular access (`access`: a handle with
/// `KEY_SET_VALUE` only saves); the file handle needs `FILE_WRITE_DATA`. A
/// deleted key is `STATUS_KEY_DELETED`.
pub fn save_key<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [kh, fh] = abi::args::<K, 2>(k, regs)?;
    let caller = k.caller();
    let (open, bytes) = k.with_state(|sys, _, _| -> Result<_> {
        require_privilege(sys, &caller, SE_BACKUP_PRIVILEGE)?;
        let (_, _, key) = super::key_of_handle(sys, &caller, Handle(kh), 0)?;
        let root = sys.registry.reg.subtree(key)?;
        let (_, cookie) = file_handle(sys, &caller, Handle(fh), FILE_WRITE_DATA)?;
        let (_, open) = disk_file(sys, cookie)?;
        Ok((open, hive::encode_regf(&HiveFile { sequence: 1, root })))
    })?;
    write_whole(&open, &bytes)?;
    Ok(STATUS_SUCCESS)
}

/// What the first step of a load found.
enum Load {
    /// The same hive file is loaded under that name already.
    Done,
    /// The file is open: read it and mount it as `name` below `parent`.
    Open { parent: KeyId, name: Vec<u16>, handle: Handle, node: NodeKey, open: Arc<OpenFile> },
}

/// Whether `attrs` names the file `node`.
fn names_file(
    sys: &mut NtSystem,
    b: &mut dyn crate::Backend,
    caller: &Caller,
    attrs: &Attributes,
    node: NodeKey,
) -> bool {
    let all = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
    let Ok((h, cookie)) = open_kernel(sys, b, caller, attrs, FILE_READ_ATTRIBUTES, all, FILE_OPEN) else {
        return false;
    };
    // An open for attributes only has no VFS file to read: its node is enough.
    let same = sys.io.file(cookie).and_then(|r| r.disk()).is_some_and(|d| d.key == node);
    close_kernel(sys, b, caller, h);
    same
}

/// `NtLoadKey(TargetKey*, SourceFile*)` (`RegLoadKeyW.yaml`):
///
/// - The target must be a new key directly below `\REGISTRY\MACHINE` or
///   `\REGISTRY\USER`: any other place (below `HKEY_CURRENT_USER`, below an
///   ordinary key, a path with more components) is `STATUS_ACCESS_DENIED`.
///   A name that exists is `STATUS_OBJECT_NAME_COLLISION`, unless it is the
///   hive loaded from the same file (`twice`: then nothing changes).
/// - The file is opened for reading and writing without sharing (another
///   open of it: `STATUS_SHARING_VIOLATION`), made if it does not exist
///   (`missing_file`: an empty hive). A file that is not a hive is
///   `STATUS_REGISTRY_IO_FAILED` and loads nothing.
pub fn load_key<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [target, source] = abi::args::<K, 2>(k, regs)?;
    let target = abi::read_attributes(k, target)?;
    let source = abi::read_attributes(k, source)?;
    let caller = k.caller();
    let now = k.system_time();
    let step = k.with_state(|sys, _, b| -> Result<Load> {
        require_privilege(sys, &caller, SE_RESTORE_PRIVILEGE)?;
        let (parent, name) = match resolve_target(sys, &caller, &target) {
            Ok(Resolved::Missing { parent, name })
                if parent == sys.registry.reg.machine() || parent == sys.registry.reg.user() =>
            {
                (parent, name)
            }
            Ok(Resolved::Found(key)) => {
                let reg = &sys.registry.reg;
                let name = reg.key(key)?.name().to_vec();
                let top = reg.child(reg.machine(), &name) == Some(key) || reg.child(reg.user(), &name) == Some(key);
                let node = reg
                    .hive_rooted_at(key)
                    .and_then(|h| sys.registry.external.iter().find(|e| e.hive == h))
                    .map(|e| e.node);
                if let Some(node) = node
                    && names_file(sys, b, &caller, &source, node)
                {
                    return Ok(Load::Done);
                }
                return Err(if top { STATUS_OBJECT_NAME_COLLISION } else { STATUS_ACCESS_DENIED });
            }
            Err(STATUS_INVALID_HANDLE) => return Err(STATUS_INVALID_HANDLE),
            _ => return Err(STATUS_ACCESS_DENIED),
        };
        let rw = GENERIC_READ | GENERIC_WRITE;
        let (handle, cookie) = open_kernel(sys, b, &caller, &source, rw, 0, FILE_OPEN_IF)?;
        match disk_file(sys, cookie) {
            Ok((node, open)) => Ok(Load::Open { parent, name, handle, node, open }),
            Err(e) => {
                close_kernel(sys, b, &caller, handle);
                Err(e)
            }
        }
    })?;
    let Load::Open { parent, name, handle, node, open } = step else { return Ok(STATUS_SUCCESS) };
    let fail = |k: &mut K, e: NtStatus| -> Result<NtStatus> {
        k.with_state(|sys, _, b| close_kernel(sys, b, &caller, handle));
        Err(e)
    };
    let bytes = match read_whole(&open) {
        Ok(b) => b,
        Err(e) => return fail(k, e),
    };
    let file = if bytes.is_empty() {
        // A new file: an empty hive, written at once so the file is one.
        let empty = HiveFile { sequence: 0, root: KeyImage { last_write: now, ..KeyImage::default() } };
        if let Err(e) = write_whole(&open, &hive::encode_regf(&empty)) {
            return fail(k, e);
        }
        empty
    } else {
        match hive::decode_regf(&bytes) {
            Ok(f) => f,
            Err(_) => return fail(k, STATUS_REGISTRY_IO_FAILED),
        }
    };
    let label = alloc::string::String::from_utf16_lossy(source.name.as_deref().unwrap_or(&[]));
    k.with_state(|sys, _, b| {
        let mounted = sys.registry.reg.mount_external(parent, &name, &label, &file, now);
        let hive = match mounted {
            Ok(h) => h,
            Err(e) => {
                close_kernel(sys, b, &caller, handle);
                return Err(e);
            }
        };
        sys.registry.external.push(External { hive, handle, node, open });
        // The log files beside the hive (empty: Poly needs none).
        if let Some(n) = source.name.as_deref() {
            for suffix in LOG_SUFFIXES {
                let mut log = source.clone();
                let mut name = n.to_vec();
                name.extend(suffix.encode_utf16());
                log.name = Some(name);
                let all = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
                if let Ok((h, _)) = open_kernel(sys, b, &caller, &log, FILE_READ_ATTRIBUTES, all, FILE_OPEN_IF) {
                    close_kernel(sys, b, &caller, h);
                }
            }
        }
        sys.registry.deliver(&mut sys.objects, Some(caller.process));
        Ok(STATUS_SUCCESS)
    })
}

/// The file bytes of external hive `hive`, marking it clean.
fn external_bytes(sys: &mut NtSystem, hive: HiveId) -> Vec<u8> {
    hive::wrap_regf(&sys.registry.reg.save(hive))
}

/// Write external hive `hive` to its file now (`NtFlushKey` of one of its
/// keys). A failed write is `STATUS_REGISTRY_IO_FAILED` and leaves it dirty.
pub fn flush_external<K: NtKernel>(k: &mut K, hive: HiveId) -> Result<()> {
    let job = k.with_state(|sys, _, _| {
        let open = sys.registry.external.iter().find(|e| e.hive == hive).map(|e| Arc::clone(&e.open))?;
        Some((open, external_bytes(sys, hive)))
    });
    let Some((open, bytes)) = job else { return Ok(()) };
    if write_whole(&open, &bytes).is_err() {
        k.with_state(|sys, _, _| sys.registry.reg.mark_hive_dirty(hive));
        return Err(STATUS_REGISTRY_IO_FAILED);
    }
    Ok(())
}

/// `NtUnloadKey(TargetKey*)` (`RegUnLoadKeyW.yaml`): the target must be the
/// root of a hive loaded by `NtLoadKey`, reached through the root it was
/// loaded under (`roots`: anything else, also an ordinary key or a system
/// hive, is `STATUS_INVALID_PARAMETER`). While a key of the hive is open the
/// call is `STATUS_ACCESS_DENIED` and the hive stays (`open_handles`). The
/// hive is written to its file, which is then closed.
pub fn unload_key<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let target = abi::arg(k, regs, 0)?;
    let target = abi::read_attributes(k, target)?;
    let caller = k.caller();
    let now = k.system_time();
    let (open, bytes) = k.with_state(|sys, _, b| -> Result<_> {
        require_privilege(sys, &caller, SE_RESTORE_PRIVILEGE)?;
        let key = match resolve_target(sys, &caller, &target) {
            Ok(Resolved::Found(key)) => key,
            Err(STATUS_INVALID_HANDLE) => return Err(STATUS_INVALID_HANDLE),
            _ => return Err(STATUS_INVALID_PARAMETER),
        };
        let hive = sys.registry.reg.hive_rooted_at(key).ok_or(STATUS_INVALID_PARAMETER)?;
        let at = sys.registry.external.iter().position(|e| e.hive == hive).ok_or(STATUS_INVALID_PARAMETER)?;
        if sys.registry.reg.hive_in_use(hive) {
            return Err(STATUS_ACCESS_DENIED);
        }
        let bytes = external_bytes(sys, hive);
        sys.registry.reg.unmount_hive(hive, now)?;
        let ext = sys.registry.external.remove(at);
        close_kernel(sys, b, &caller, ext.handle);
        sys.registry.deliver(&mut sys.objects, Some(caller.process));
        Ok((ext.open, bytes))
    })?;
    write_whole(&open, &bytes)?;
    Ok(STATUS_SUCCESS)
}
