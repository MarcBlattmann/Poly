//! The environment a new NT process gets (ADR 0072), and the environment
//! block format (Learn: "Environment Variables", "Changing Environment
//! Variables", `CreateProcessW`'s `lpEnvironment`,
//! `ExpandEnvironmentStringsW`).
//!
//! **The block.** A Unicode environment block is a sequence of
//! `Name=Value\0` strings ended by one more `\0`. Names are compared
//! without case. Learn ("Changing Environment Variables"): "All strings in
//! the environment block must be sorted alphabetically by name. The sort is
//! case-insensitive, Unicode order, without regard to locale" — so
//! [`Block::to_utf16`] sorts by the upcased UTF-16 names (the object
//! manager's upcase rule, ADR 0030).
//!
//! **Expansion** (`ExpandEnvironmentStringsW`): each `%Name%` of a
//! defined variable is replaced by its value; an undefined one is left as
//! it is (Learn: "If the name is not found, the %variableName% portion is
//! left unexpanded").
//!
//! **What a process gets** ([`build`]), in this order, each later step
//! overriding an earlier one's variable of the same name:
//!
//! 1. The machine's variables Windows sets for every process (Learn:
//!    "Recognized environment variables", "WOW64 Implementation Details"
//!    for a 64-bit process): `SystemDrive`, `SystemRoot`, `ProgramFiles`,
//!    `ProgramW6432`, `ProgramFiles(x86)`, `CommonProgramFiles`,
//!    `CommonProgramW6432`, `CommonProgramFiles(x86)` (from the
//!    `ProgramFilesDir` family under `HKLM\SOFTWARE\Microsoft\Windows\
//!    CurrentVersion`), `ProgramData`, `ALLUSERSPROFILE`, `PUBLIC` (from
//!    `ProfileList`), `COMPUTERNAME` (the `ComputerName` key) and
//!    `NUMBER_OF_PROCESSORS` (the kernel's count).
//! 2. The system variables: `HKLM\SYSTEM\CurrentControlSet\Control\
//!    Session Manager\Environment` (Learn: "Environment Variables").
//! 3. The user's logon variables: `HKCU\Volatile Environment`
//!    (`USERPROFILE`, `HOMEDRIVE`, `HOMEPATH`, `APPDATA`, `LOCALAPPDATA`,
//!    `USERNAME`, `USERDOMAIN`, `LOGONSERVER`).
//! 4. The user's variables: `HKCU\Environment` (Learn: "Environment
//!    Variables"). The user's `Path` is appended to the system's with a `;`
//!    rather than replacing it (to be confirmed by the spec agent).
//!
//! A `REG_EXPAND_SZ` value is expanded against what is set so far; other
//! value types than `REG_SZ` and `REG_EXPAND_SZ` are skipped. Where a
//! registry value is missing, the layout's own value ([`super`]) stands in,
//! so a process always gets every variable.

use super::{
    COMMON_FILES, COMMON_FILES_X86, PROGRAM_DATA, PROGRAM_FILES, PROGRAM_FILES_X86, PUBLIC, SYSTEM_DRIVE, SYSTEM_ROOT,
    User,
};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cmp::Ordering;
use poly_ntobj::name::{upcase, wide};
use poly_registry::Registry;
use poly_registry::consts::{REG_DWORD, REG_EXPAND_SZ, REG_SZ};
use poly_registry::tree::KeyId;

/// A set of environment variables, names without case.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Block {
    vars: Vec<(String, String)>,
}

/// The upcased UTF-16 form of a name, for comparing and sorting.
fn key(name: &str) -> Vec<u16> {
    name.encode_utf16().map(upcase).collect()
}

fn same_name(a: &str, b: &str) -> bool {
    key(a) == key(b)
}

impl Block {
    pub fn new() -> Block {
        Block::default()
    }

    /// The value of `name`, found without case.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars.iter().find(|(n, _)| same_name(n, name)).map(|(_, v)| v.as_str())
    }

    /// Set `name` to `value`. An existing variable of that name (without
    /// case) keeps its name's case and gets the new value.
    pub fn set(&mut self, name: &str, value: &str) {
        match self.vars.iter_mut().find(|(n, _)| same_name(n, name)) {
            Some((_, v)) => *v = value.into(),
            None => self.vars.push((name.into(), value.into())),
        }
    }

    pub fn remove(&mut self, name: &str) {
        self.vars.retain(|(n, _)| !same_name(n, name));
    }

    pub fn len(&self) -> usize {
        self.vars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }

    /// The variables sorted as the block holds them.
    pub fn sorted(&self) -> Vec<(&str, &str)> {
        let mut v: Vec<(&str, &str)> = self.vars.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
        v.sort_by(|a, b| compare_names(a.0, b.0));
        v
    }

    /// `ExpandEnvironmentStringsW`: replace each `%Name%` of a defined
    /// variable; leave undefined ones as they are.
    pub fn expand(&self, s: &str) -> String {
        let mut out = String::new();
        let mut rest = s;
        while let Some(start) = rest.find('%') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let Some(end) = after.find('%') else {
                out.push_str(&rest[start..]);
                return out;
            };
            let name = &after[..end];
            match (!name.is_empty()).then(|| self.get(name)).flatten() {
                Some(value) => {
                    out.push_str(value);
                    rest = &after[end + 1..];
                }
                None => {
                    // Keep `%name` and look again from the closing `%`,
                    // which may open the next reference.
                    out.push('%');
                    out.push_str(name);
                    rest = &after[end..];
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// The block as a process receives it: sorted `Name=Value\0` strings
    /// and a final `\0` (two NULs for an empty block).
    pub fn to_utf16(&self) -> Vec<u16> {
        let mut out = Vec::new();
        for (name, value) in self.sorted() {
            out.extend(name.encode_utf16());
            out.push(u16::from(b'='));
            out.extend(value.encode_utf16());
            out.push(0);
        }
        if out.is_empty() {
            out.push(0);
        }
        out.push(0);
        out
    }

    /// Read a block (up to its terminating empty string). A string without
    /// `=` after its first character is skipped; a leading `=` belongs to
    /// the name (the per-drive current directories, `=C:=C:\...`).
    pub fn from_utf16(block: &[u16]) -> Block {
        let mut b = Block::new();
        for s in block.split(|&c| c == 0) {
            if s.is_empty() {
                break;
            }
            let text = String::from_utf16_lossy(s);
            if let Some(eq) = text.char_indices().skip(1).find(|&(_, c)| c == '=').map(|(i, _)| i) {
                b.vars.push((text[..eq].to_string(), text[eq + 1..].to_string()));
            }
        }
        b
    }
}

/// Learn's order: without case, by UTF-16 code unit.
pub fn compare_names(a: &str, b: &str) -> Ordering {
    key(a).cmp(&key(b))
}

/// What the kernel tells [`build`] about the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Machine {
    /// `NUMBER_OF_PROCESSORS`.
    pub processors: u32,
}

/// A `REG_SZ` / `REG_EXPAND_SZ` value's text: UTF-16LE, up to the first
/// NUL.
fn text_of(data: &[u8]) -> String {
    let units: Vec<u16> =
        data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    String::from_utf16_lossy(&units)
}

/// The string value `name` of the key at `path` below `start`: its type
/// and text.
pub fn string_value(reg: &Registry, start: KeyId, path: &str, name: &str) -> Option<(u32, String)> {
    let key = reg.find(start, path)?;
    let v = reg.value(key, &wide(name)).ok()?;
    matches!(v.kind, REG_SZ | REG_EXPAND_SZ).then(|| (v.kind, text_of(&v.data)))
}

/// `SYSTEM\ControlSet00n`, the control set `SYSTEM\Select`'s `Current`
/// names (what the `CurrentControlSet` link points at); `ControlSet001`
/// when there is none.
pub fn current_control_set(reg: &Registry) -> String {
    let current = reg
        .find(reg.machine(), "SYSTEM\\Select")
        .and_then(|k| reg.value(k, &wide("Current")).ok())
        .filter(|v| v.kind == REG_DWORD && v.data.len() == 4)
        .map_or(1, |v| u32::from_le_bytes([v.data[0], v.data[1], v.data[2], v.data[3]]));
    alloc::format!("SYSTEM\\ControlSet{current:03}")
}

/// `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion`.
pub const CURRENT_VERSION: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion";
/// `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion`.
pub const NT_CURRENT_VERSION: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";
/// `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList`.
pub const PROFILE_LIST: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\ProfileList";

/// Every key's values of type `REG_SZ` / `REG_EXPAND_SZ`, set into `b`
/// (expanded against `b` as it is). The `Path` of `append_path` is joined
/// to an existing `Path` with `;`.
fn apply_key(b: &mut Block, reg: &Registry, start: KeyId, path: &str, append_path: bool) {
    let Some(key) = reg.find(start, path) else { return };
    let Ok(k) = reg.key(key) else { return };
    for v in k.values() {
        if !matches!(v.kind, REG_SZ | REG_EXPAND_SZ) || v.name.is_empty() {
            continue;
        }
        let name = String::from_utf16_lossy(&v.name);
        let raw = text_of(&v.data);
        let value = if v.kind == REG_EXPAND_SZ { b.expand(&raw) } else { raw };
        if append_path && same_name(&name, "Path") {
            if let Some(old) = b.get("Path").filter(|p| !p.is_empty()) {
                let joined =
                    if old.ends_with(';') { alloc::format!("{old}{value}") } else { alloc::format!("{old};{value}") };
                b.set(&name, &joined);
                continue;
            }
        }
        b.set(&name, &value);
    }
}

/// The default environment of a new NT process of `user` (see the module
/// documentation for the order).
pub fn build(reg: &Registry, user: &User, machine: Machine) -> Block {
    let m = reg.machine();
    let mut b = Block::new();
    let machine_value = |b: &mut Block, var: &str, path: &str, name: &str, fallback: &str| {
        let value = match string_value(reg, m, path, name) {
            Some((REG_EXPAND_SZ, raw)) => b.expand(&raw),
            Some((_, raw)) => raw,
            None => fallback.into(),
        };
        b.set(var, &value);
    };
    // 1. The machine's variables.
    b.set("SystemDrive", SYSTEM_DRIVE);
    machine_value(&mut b, "SystemRoot", NT_CURRENT_VERSION, "SystemRoot", SYSTEM_ROOT);
    machine_value(&mut b, "ProgramFiles", CURRENT_VERSION, "ProgramFilesDir", PROGRAM_FILES);
    machine_value(&mut b, "ProgramW6432", CURRENT_VERSION, "ProgramW6432Dir", PROGRAM_FILES);
    machine_value(&mut b, "ProgramFiles(x86)", CURRENT_VERSION, "ProgramFilesDir (x86)", PROGRAM_FILES_X86);
    machine_value(&mut b, "CommonProgramFiles", CURRENT_VERSION, "CommonFilesDir", COMMON_FILES);
    machine_value(&mut b, "CommonProgramW6432", CURRENT_VERSION, "CommonW6432Dir", COMMON_FILES);
    machine_value(&mut b, "CommonProgramFiles(x86)", CURRENT_VERSION, "CommonFilesDir (x86)", COMMON_FILES_X86);
    machine_value(&mut b, "ProgramData", PROFILE_LIST, "ProgramData", PROGRAM_DATA);
    let program_data = b.get("ProgramData").unwrap_or(PROGRAM_DATA).to_string();
    b.set("ALLUSERSPROFILE", &program_data);
    machine_value(&mut b, "PUBLIC", PROFILE_LIST, "Public", PUBLIC);
    let control = current_control_set(reg);
    let computer = alloc::format!("{control}\\Control\\ComputerName\\ComputerName");
    machine_value(&mut b, "COMPUTERNAME", &computer, "ComputerName", poly_registry::defaults::POLY_COMPUTER_NAME);
    b.set("NUMBER_OF_PROCESSORS", &alloc::format!("{}", machine.processors.max(1)));

    // 2. The system variables.
    apply_key(&mut b, reg, m, &alloc::format!("{control}\\Control\\Session Manager\\Environment"), false);

    // 3. The user's logon variables, with the layout's as fallbacks.
    let profile = user.profile();
    for (var, fallback) in [
        ("USERPROFILE", profile.as_str()),
        ("HOMEDRIVE", SYSTEM_DRIVE),
        ("HOMEPATH", user.home_path().as_str()),
        ("APPDATA", user.app_data().as_str()),
        ("LOCALAPPDATA", user.local_app_data().as_str()),
        ("USERNAME", user.name),
    ] {
        b.set(var, fallback);
    }
    let u = reg.user();
    apply_key(&mut b, reg, u, &alloc::format!("{}\\Volatile Environment", user.sid), false);

    // 4. The user's variables.
    apply_key(&mut b, reg, u, &alloc::format!("{}\\Environment", user.sid), true);
    b
}
