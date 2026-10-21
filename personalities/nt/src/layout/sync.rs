//! The registry values that must agree with the layout (ADR 0072).
//!
//! The default hives (`poly_registry::defaults`, ADR 0066) already hold
//! most of them; [`sync`] runs at every boot after the hives are loaded and
//! makes sure each one is there and right, so an older hive file, or one a
//! program damaged, comes back in line. Two policies:
//!
//! - [`Policy::Fixed`]: the layout decides; a different value is
//!   rewritten. These name folders Poly makes and does not move: the
//!   program-files family, `SystemRoot`, `ProfileList`, the user's logon
//!   variables, and the `Shell Folders` mirrors.
//! - [`Policy::Default`]: written only when missing, because a user or
//!   program may legitimately change it: `User Shell Folders` (folder
//!   redirection) and the `TEMP` / `TMP` / `windir` variables.
//!
//! `Shell Folders` holds the expanded form of whatever `User Shell Folders`
//! says (so a redirected `Personal` shows in both), which is why it is
//! written after `User Shell Folders` and read back from it.
//!
//! A value already equal is not written, so a boot with nothing to change
//! leaves every hive clean (nothing is saved).

use super::env::{Block, CURRENT_VERSION, NT_CURRENT_VERSION, PROFILE_LIST, current_control_set};
use super::{
    Base, COMMON_FILES, COMMON_FILES_X86, FOLDERS, PROGRAM_DATA, PROGRAM_FILES, PROGRAM_FILES_X86, PUBLIC,
    SYSTEM_DRIVE, SYSTEM_ROOT, User,
};
use alloc::string::String;
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::name::wide;
use poly_registry::Registry;
use poly_registry::consts::{REG_EXPAND_SZ, REG_SZ};
use poly_registry::defaults::sz;
use poly_registry::tree::{CreateOptions, KeyId};

/// Whether a value the layout owns is rewritten when it differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    Fixed,
    Default,
}

/// Which master key a value's key is below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Under {
    /// `\REGISTRY\MACHINE`.
    Machine,
    /// `\REGISTRY\USER`.
    Users,
}

/// One value the layout owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub under: Under,
    /// The key below the master key.
    pub key: String,
    /// Made volatile if missing (the user's `Volatile Environment`).
    pub volatile: bool,
    pub name: String,
    /// `REG_SZ` or `REG_EXPAND_SZ`.
    pub kind: u32,
    pub data: String,
    pub policy: Policy,
}

/// `...\Explorer\User Shell Folders` and `...\Explorer\Shell Folders`.
pub const USER_SHELL_FOLDERS: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\User Shell Folders";
pub const SHELL_FOLDERS: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Shell Folders";
/// The machine's (common) shell folders, below `HKLM`.
pub const MACHINE_USER_SHELL_FOLDERS: &str =
    "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\User Shell Folders";
pub const MACHINE_SHELL_FOLDERS: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Shell Folders";

fn entry(under: Under, key: &str, name: &str, kind: u32, data: &str, policy: Policy) -> Entry {
    Entry { under, key: key.into(), volatile: false, name: name.into(), kind, data: data.into(), policy }
}

/// The values the layout owns, but the `Shell Folders` mirrors (made from
/// what `User Shell Folders` holds, [`sync`]).
pub fn entries(reg: &Registry, user: &User) -> Vec<Entry> {
    use Policy::*;
    use Under::*;
    let mut v = Vec::new();
    v.push(entry(Machine, NT_CURRENT_VERSION, "SystemRoot", REG_SZ, SYSTEM_ROOT, Fixed));
    for (name, data) in [
        ("ProgramFilesDir", PROGRAM_FILES),
        ("ProgramW6432Dir", PROGRAM_FILES),
        ("ProgramFilesDir (x86)", PROGRAM_FILES_X86),
        ("CommonFilesDir", COMMON_FILES),
        ("CommonW6432Dir", COMMON_FILES),
        ("CommonFilesDir (x86)", COMMON_FILES_X86),
    ] {
        v.push(entry(Machine, CURRENT_VERSION, name, REG_SZ, data, Fixed));
    }
    for (name, data) in [
        ("ProfilesDirectory", "%SystemDrive%\\Users"),
        ("Default", "%SystemDrive%\\Users\\Default"),
        ("Public", "%SystemDrive%\\Users\\Public"),
        ("ProgramData", "%SystemDrive%\\ProgramData"),
    ] {
        v.push(entry(Machine, PROFILE_LIST, name, REG_EXPAND_SZ, data, Fixed));
    }
    let profile = user.profile();
    let me = alloc::format!("{PROFILE_LIST}\\{}", user.sid);
    v.push(entry(Machine, &me, "ProfileImagePath", REG_EXPAND_SZ, &profile, Fixed));

    let env = alloc::format!("{}\\Control\\Session Manager\\Environment", current_control_set(reg));
    for (name, data) in [("TEMP", "%SystemRoot%\\TEMP"), ("TMP", "%SystemRoot%\\TEMP"), ("windir", "%SystemRoot%")] {
        v.push(entry(Machine, &env, name, REG_EXPAND_SZ, data, Default));
    }
    let user_env = alloc::format!("{}\\Environment", user.sid);
    for name in ["TEMP", "TMP"] {
        v.push(entry(Users, &user_env, name, REG_EXPAND_SZ, "%USERPROFILE%\\AppData\\Local\\Temp", Default));
    }
    let volatile = alloc::format!("{}\\Volatile Environment", user.sid);
    for (name, data) in [
        ("USERPROFILE", profile.clone()),
        ("HOMEDRIVE", SYSTEM_DRIVE.into()),
        ("HOMEPATH", user.home_path()),
        ("APPDATA", user.app_data()),
        ("LOCALAPPDATA", user.local_app_data()),
        ("USERNAME", user.name.into()),
    ] {
        let mut e = entry(Users, &volatile, name, REG_SZ, &data, Fixed);
        e.volatile = true;
        v.push(e);
    }

    let user_usf = alloc::format!("{}\\{USER_SHELL_FOLDERS}", user.sid);
    for f in FOLDERS {
        let Some(shell) = f.shell else { continue };
        let (under, key) = match f.base {
            Base::Drive => (Machine, MACHINE_USER_SHELL_FOLDERS),
            Base::Profile => (Users, user_usf.as_str()),
        };
        v.push(entry(under, key, shell, REG_EXPAND_SZ, &f.unexpanded(user), Default));
    }
    v
}

/// The variables `User Shell Folders` values are expanded with.
pub fn expansion_block(user: &User) -> Block {
    let mut b = Block::new();
    b.set("SystemDrive", SYSTEM_DRIVE);
    b.set("SystemRoot", SYSTEM_ROOT);
    b.set("ProgramData", PROGRAM_DATA);
    b.set("ALLUSERSPROFILE", PROGRAM_DATA);
    b.set("PUBLIC", PUBLIC);
    b.set("USERPROFILE", &user.profile());
    b.set("APPDATA", &user.app_data());
    b.set("LOCALAPPDATA", &user.local_app_data());
    b
}

fn master(reg: &Registry, under: Under) -> KeyId {
    match under {
        Under::Machine => reg.machine(),
        Under::Users => reg.user(),
    }
}

/// Open or make each component of `path` below `start`.
fn make_key(reg: &mut Registry, start: KeyId, path: &str, volatile: bool, now: u64) -> Result<KeyId, NtStatus> {
    let mut cur = start;
    for c in path.split('\\').filter(|c| !c.is_empty()) {
        // An existing key is opened whatever its volatility; a new one is
        // volatile only when asked for (the last component).
        let opts = CreateOptions { volatile, ..CreateOptions::default() };
        cur = match reg.child(cur, &wide(c)) {
            Some(k) => k,
            None => reg.create_key(cur, &wide(c), opts, now)?.0,
        };
    }
    Ok(cur)
}

/// What [`sync`] changed: `<key>\<value>` of each value it wrote.
pub type Changed = Vec<String>;

/// Write `e` if its policy says so. Returns whether it wrote.
fn apply(reg: &mut Registry, e: &Entry, now: u64) -> Result<bool, NtStatus> {
    let start = master(reg, e.under);
    let key = make_key(reg, start, &e.key, e.volatile, now)?;
    let data = sz(&e.data);
    let name = wide(&e.name);
    match reg.value(key, &name) {
        Ok(v) if v.kind == e.kind && v.data == data => return Ok(false),
        Ok(_) if e.policy == Policy::Default => return Ok(false),
        _ => {}
    }
    reg.set_value(key, &name, e.kind, &data, now)?;
    Ok(true)
}

/// Bring the registry in line with the layout for `user` (see the module
/// documentation). Idempotent: a second call changes nothing.
pub fn sync(reg: &mut Registry, user: &User, now: u64) -> Result<Changed, NtStatus> {
    let mut changed = Vec::new();
    for e in entries(reg, user) {
        if apply(reg, &e, now)? {
            changed.push(alloc::format!("{}\\{}", e.key, e.name));
        }
    }
    // The Shell Folders mirrors: the expanded User Shell Folders values.
    let expand = expansion_block(user);
    let user_sf = alloc::format!("{}\\{SHELL_FOLDERS}", user.sid);
    let user_usf = alloc::format!("{}\\{USER_SHELL_FOLDERS}", user.sid);
    for f in FOLDERS {
        let Some(shell) = f.shell else { continue };
        let (under, usf, sf) = match f.base {
            Base::Drive => (Under::Machine, MACHINE_USER_SHELL_FOLDERS, MACHINE_SHELL_FOLDERS),
            Base::Profile => (Under::Users, user_usf.as_str(), user_sf.as_str()),
        };
        let start = master(reg, under);
        let source =
            super::env::string_value(reg, start, usf, shell).map_or_else(|| f.unexpanded(user), |(_, raw)| raw);
        let e = entry(under, sf, shell, REG_SZ, &expand.expand(&source), Policy::Fixed);
        if apply(reg, &e, now)? {
            changed.push(alloc::format!("{}\\{}", e.key, e.name));
        }
    }
    Ok(changed)
}
