//! The kernel's side of the Windows file system layout ([ADR 0072]).
//!
//! The table, the case-insensitive creation, the registry sync and the
//! environment block are `poly_personality_nt::layout` (host-tested); this
//! file only decides where `C:` lives and runs them once, when the NT state
//! is first made:
//!
//! - **Where `C:` lives.** On the disk that keeps the registry hives
//!   (ADR 0066: the first writable disk with a `/poly` directory), in
//!   `<disk>/poly/nt/c`, so installed programs and the user's files
//!   survive a reboot; `/nt/c` (ADR 0067's path for `C:`) is then a
//!   symbolic link to it, so ADR 0067's mapping stays true and Linux
//!   programs find `C:` at one path. With no such disk `C:` is `/nt/c` on
//!   the boot file system and lasts one boot, like the in-memory registry.
//! - **The folders** are made (or found, without case) at every first NT
//!   use of a boot; then the registry values that must agree with them are
//!   synced (written only where they differ).
//!
//! [ADR 0072]: ../../../docs/adr/0072-windows-file-system-layout.md

use crate::fs;
use crate::sync::Mutex;
use alloc::string::String;
use alloc::vec::Vec;
use poly_personality_nt::io::VfsRoot;
use poly_personality_nt::layout::{self, POLY_USER, create, env, sync};
use poly_personality_nt::system::NtSystem;
use poly_vfs::{At, Cred, Errno, FileType};

/// The VFS path ADR 0067 maps `\??\C:` to.
pub const C_PATH: &str = "/nt/c";

/// The directory `C:` really is (after following `/nt/c`), once decided.
static C_ROOT: Mutex<Option<String>> = Mutex::new(None);

/// The VFS directories that are `C:`: `/nt/c` and, when it is a link, its
/// target (for naming executables, [`layout::dos_path_of_vfs`]).
pub fn c_roots() -> Vec<String> {
    let mut v = alloc::vec![String::from(C_PATH)];
    if let Some(r) = C_ROOT.lock().clone()
        && r != C_PATH
    {
        v.push(r);
    }
    v
}

fn mkdir_all(path: &str) -> Result<(), Errno> {
    let vfs = fs::vfs();
    let ctx = vfs.root_context()?;
    let mut at = String::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        at.push('/');
        at.push_str(part);
        match vfs.mkdir(&ctx, &Cred::root(), At::Cwd, at.as_bytes(), 0o755) {
            Ok(()) | Err(Errno::EEXIST) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Point `/nt/c` at `target` (a directory on the hive disk), unless
/// something else is already there: an empty directory is replaced, a link
/// to `target` kept; anything else is kept and `C:` stays on it.
fn link_c(target: &str) -> Result<(), Errno> {
    let vfs = fs::vfs();
    let ctx = vfs.root_context()?;
    let cred = Cred::root();
    mkdir_all(target)?;
    mkdir_all("/nt")?;
    match vfs.lookup(&ctx, &cred, At::Cwd, C_PATH.as_bytes(), false) {
        Err(Errno::ENOENT) => {}
        Err(e) => return Err(e),
        Ok(node) => match node.kind()? {
            FileType::Symlink if node.readlink()? == target.as_bytes() => return Ok(()),
            FileType::Directory => vfs.rmdir(&ctx, &cred, At::Cwd, C_PATH.as_bytes())?,
            _ => return Err(Errno::EEXIST),
        },
    }
    vfs.symlink(&ctx, &cred, target.as_bytes(), At::Cwd, C_PATH.as_bytes())
}

/// Make the layout and sync the registry (once, as the NT state is made,
/// before file I/O attaches `C:`).
pub(super) fn prepare(sys: &mut NtSystem) {
    // Where the hives live decides where C: lives.
    if let Some(hives) = super::registry::store_dir() {
        let disk_c = alloc::format!("{}/c", hives.trim_end_matches('/').trim_end_matches("/registry"));
        if let Err(e) = link_c(&disk_c) {
            crate::kprintln!("nt: layout: {C_PATH} not linked to {disk_c} ({e:?}); C: stays on {C_PATH}");
        }
    } else if let Err(e) = mkdir_all(C_PATH) {
        crate::kprintln!("nt: layout: cannot make {C_PATH}: {e:?}");
        return;
    }
    let vfs = fs::vfs();
    let Ok(ctx) = vfs.root_context() else { return };
    let cred = Cred::root();
    let Ok(root) = vfs.lookup(&ctx, &cred, At::Cwd, C_PATH.as_bytes(), true) else {
        crate::kprintln!("nt: layout: {C_PATH} is not there");
        return;
    };
    let real = vfs.path_of(&ctx, &root).ok().map(|p| String::from_utf8_lossy(&p).into_owned());
    *C_ROOT.lock() = real.clone();
    let vr = VfsRoot { vfs, ctx, cred };
    let report = create::create(&vr, &root, &POLY_USER);
    for (path, status) in &report.conflicts {
        crate::kprintln!("nt: layout: {path} cannot be made ({:#x}): something else is there", status.0);
    }
    let synced = sync::sync(&mut sys.registry.reg, &POLY_USER, super::registry::filetime_now());
    crate::kprintln!(
        "nt: C: is {}; {} folders made, {} registry values synced",
        real.as_deref().unwrap_or(C_PATH),
        report.created.len(),
        synced.as_ref().map_or(0, Vec::len)
    );
    if let Err(e) = synced {
        crate::kprintln!("nt: layout: registry sync failed: {:#x}", e.0);
    }
}

/// The environment block a new NT process gets (from the registry, under
/// the NT lock).
pub(super) fn environment(sys: &NtSystem) -> Vec<u16> {
    let processors = crate::thread::online_cpus().len() as u32;
    env::build(&sys.registry.reg, &POLY_USER, env::Machine { processors }).to_utf16()
}

/// The DOS path of an executable at VFS path `path`.
pub fn dos_path(path: &[u8]) -> String {
    let roots = c_roots();
    let roots: Vec<&[u8]> = roots.iter().map(|r| r.as_bytes()).collect();
    let mut abs = Vec::new();
    if !path.starts_with(b"/") {
        abs.push(b'/');
    }
    abs.extend_from_slice(path);
    layout::dos_path_of_vfs(&abs, &roots)
}
