//! The NT registry's hive files (ADR 0066).
//!
//! The registry logic is `poly-registry` (host-tested); the NT personality
//! reaches it through `\REGISTRY`. This module only decides where hive
//! files live and moves their bytes:
//!
//! - **Where.** The first writable disk mount (`fs::disk::mounts`, in mount
//!   order) whose root has a `poly` directory keeps the hives in
//!   `<mount>/poly/nt/registry/<NAME>.hive` (`cargo xtask root-disk` makes
//!   that directory on the persistent root disk, ADR 0059). With no such
//!   disk when the NT state is first made, the registry lives in memory
//!   only for the whole boot. The choice is made once.
//! - **Load.** At the first NT use: each default hive is read from its
//!   file; a missing file means first boot (the defaults are made), a
//!   corrupt one is logged and replaced by the defaults.
//! - **Save.** `NtFlushKey` writes its hive at once; every hive with
//!   unsaved changes is written when an NT process ends and before the
//!   machine goes down ([`flush`], from `power::prepare_shutdown`). A save
//!   writes `<NAME>.hive.new`, syncs the volume and renames it over
//!   `<NAME>.hive`, so a crash leaves the old or the new file, never half
//!   of one. The bytes are made under the NT lock and written after it is
//!   released.

use crate::fs;
use crate::sync::Mutex;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_personality_nt::system::NtSystem;
use poly_registry::consts::{FILETIME_UNIX_EPOCH, STATUS_REGISTRY_IO_FAILED};
use poly_registry::defaults::{self, HIVES, Outcome};
use poly_vfs::{At, Cred, OpenFlags, RenameFlags};

/// Below a disk's root: where the hive files go.
pub const DIR: &str = "poly/nt/registry";
/// A hive file's extension.
pub const EXTENSION: &str = ".hive";

/// The hive directory: `None` until decided, then `Some(None)` for memory
/// only or `Some(Some(dir))`.
static STORE: Mutex<Option<Option<String>>> = Mutex::new(None);

/// The system time as a `FILETIME`.
pub fn filetime_now() -> u64 {
    crate::time::realtime_ns() / 100 + FILETIME_UNIX_EPOCH
}

fn is_dir(path: &str) -> bool {
    let vfs = fs::vfs();
    let Ok(ctx) = vfs.root_context() else { return false };
    vfs.stat(&ctx, &Cred::root(), At::Cwd, path.as_bytes(), true).is_ok_and(|s| s.mode & 0o170_000 == 0o040_000)
}

/// Decide (once) where the hives live.
pub fn store_dir() -> Option<String> {
    let mut store = STORE.lock();
    store
        .get_or_insert_with(|| {
            fs::disk::mounts().into_iter().filter(|m| !m.read_only).find_map(|m| {
                let base = m.path.trim_end_matches('/');
                is_dir(&format!("{base}/poly")).then(|| format!("{base}/{DIR}"))
            })
        })
        .clone()
}

/// Read a whole file, `None` if it cannot be opened or read.
pub fn read_file(path: &str) -> Option<Vec<u8>> {
    let vfs = fs::vfs();
    let ctx = vfs.root_context().ok()?;
    let file = vfs.open(&ctx, &Cred::root(), At::Cwd, path.as_bytes(), OpenFlags::RDONLY, 0).ok()?;
    let mut out = Vec::new();
    let mut chunk = alloc::vec![0u8; 64 * 1024];
    loop {
        match file.read_at(out.len() as u64, &mut chunk) {
            Ok(0) => return Some(out),
            Ok(n) => out.extend_from_slice(&chunk[..n]),
            Err(_) => return None,
        }
    }
}

fn mkdir_all(path: &str) -> Result<(), poly_vfs::Errno> {
    let vfs = fs::vfs();
    let ctx = vfs.root_context()?;
    let mut at = String::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        at.push('/');
        at.push_str(part);
        match vfs.mkdir(&ctx, &Cred::root(), At::Cwd, at.as_bytes(), 0o700) {
            Ok(()) | Err(poly_vfs::Errno::EEXIST) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Write `data` as `<dir>/<file>.hive`, atomically (see the module
/// documentation).
pub fn save_in(dir: &str, file: &str, data: &[u8]) -> Result<(), poly_vfs::Errno> {
    let step = |what: &'static str| {
        move |e: poly_vfs::Errno| {
            crate::kprintln!("nt: registry: saving {dir}/{file}{EXTENSION}: {what} failed: {e:?}");
            e
        }
    };
    mkdir_all(dir).map_err(step("making the directory"))?;
    let vfs = fs::vfs();
    let ctx = vfs.root_context()?;
    let cred = Cred::root();
    let path = format!("{dir}/{file}{EXTENSION}");
    let new = format!("{path}.new");
    let flags = OpenFlags::WRONLY | OpenFlags::CREAT | OpenFlags::TRUNC;
    let f = vfs.open(&ctx, &cred, At::Cwd, new.as_bytes(), flags, 0o600).map_err(step("creating the .new file"))?;
    let mut done = 0;
    while done < data.len() {
        match f.write_at(done as u64, &data[done..]).map_err(step("writing"))? {
            0 => return Err(poly_vfs::Errno::EIO),
            n => done += n,
        }
    }
    f.sync().map_err(step("syncing"))?;
    drop(f);
    vfs.rename(&ctx, &cred, At::Cwd, new.as_bytes(), At::Cwd, path.as_bytes(), RenameFlags::default())
        .map_err(step("renaming"))?;
    Ok(())
}

/// `<dir>/<file>.hive`, if it can be read.
pub fn load_in(dir: &str, file: &str) -> Option<Vec<u8>> {
    read_file(&format!("{dir}/{file}{EXTENSION}"))
}

/// Save hive `file` where the hives live (`NtFlushKey`); nothing to do in
/// memory-only mode.
pub fn save(file: &str, data: &[u8]) -> Result<(), NtStatus> {
    let Some(dir) = store_dir() else { return Ok(()) };
    save_in(&dir, file, data).map_err(|e| {
        crate::kprintln!("nt: registry: writing hive {file} to {dir} failed: {e:?}");
        STATUS_REGISTRY_IO_FAILED
    })
}

/// The NT system as the first NT use makes it: the default hives, each
/// loaded from its file when there is one.
pub fn boot_system() -> NtSystem {
    let dir = store_dir();
    let mut reg = poly_registry::Registry::new();
    let outcomes = defaults::install(&mut reg, filetime_now(), &mut |f| dir.as_deref().and_then(|d| load_in(d, f)))
        .expect("the default hives fit an empty registry");
    match &dir {
        Some(d) => crate::kprintln!("nt: registry hives in {d}"),
        None => crate::kprintln!("nt: registry in memory (no disk with a /poly directory)"),
    }
    for (spec, outcome) in HIVES.iter().zip(outcomes) {
        if let Outcome::Corrupt(status) = outcome {
            crate::kprintln!("nt: registry: hive {} is corrupt ({status:?}); using the defaults", spec.file);
        }
    }
    NtSystem::with_registry(reg)
}

/// Write every hive with unsaved changes (an NT process ended, or the
/// machine is going down). Errors are logged and the hive stays dirty.
pub fn flush() {
    if super::with_existing(|_| ()).is_none() {
        return;
    }
    let Some(dir) = store_dir() else { return };
    let saves = super::with_existing(|s| {
        let reg = &mut s.sys.registry.reg;
        reg.dirty_hives().into_iter().map(|h| (h, reg.hive(h).file.clone(), reg.save(h))).collect::<Vec<_>>()
    });
    for (hive, file, bytes) in saves.unwrap_or_default() {
        if let Err(e) = save_in(&dir, &file, &bytes) {
            crate::kprintln!("nt: registry: writing hive {file} to {dir} failed: {e:?}");
            super::with_existing(|s| s.sys.registry.reg.mark_hive_dirty(hive));
        }
    }
}
