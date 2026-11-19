//! The default environment of a user's new process (ADR 0073), one rule for the kernel (the
//! environment of the first process, `poly_personality_nt::layout::env`) and for `userenv.dll`'s
//! `CreateEnvironmentBlock` (Learn: "CreateEnvironmentBlock", "Environment Variables", "User
//! Environment Block", "Recognized environment variables"; the sheet
//! `specs/windows/userenv.dll/CreateEnvironmentBlock.yaml`). The registry is read through a
//! [`Source`]: the kernel's own tree, or the registry functions in user mode.
//!
//! **The block.** A Unicode environment block is a sequence of `Name=Value\0` strings ended by
//! one more `\0`. Names are compared without case. Learn ("Changing Environment Variables"):
//! "All strings in the environment block must be sorted alphabetically by name. The sort is
//! case-insensitive, Unicode order, without regard to locale" — so [`Block::to_utf16`] sorts by
//! the upcased UTF-16 names (the object manager's upcase rule, ADR 0030).
//!
//! **Expansion** (`ExpandEnvironmentStringsW`): each `%Name%` of a defined variable is replaced
//! by its value; an undefined one is left as it is (Learn: "If the name is not found, the
//! %variableName% portion is left unexpanded").
//!
//! **What a process gets** ([`build`]), in this order, each later step overriding an earlier
//! one's variable of the same name:
//!
//! 0. The block it starts from: empty for a new process; the caller's own environment for
//!    `CreateEnvironmentBlock(.., bInherit = TRUE)` (the sheet: the caller's variables and its
//!    `=C:` entries appear, but the values of the steps below win).
//! 1. The machine's variables Windows sets for every process (Learn: "Recognized environment
//!    variables", "WOW64 Implementation Details" for a 64-bit process): `SystemDrive`,
//!    `SystemRoot`, `ProgramFiles`, `ProgramW6432`, `ProgramFiles(x86)`, `CommonProgramFiles`,
//!    `CommonProgramW6432`, `CommonProgramFiles(x86)` (from the `ProgramFilesDir` family under
//!    `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion`), `ProgramData`, `ALLUSERSPROFILE`,
//!    `PUBLIC` (from `ProfileList`), `COMPUTERNAME` (the `ComputerName` key),
//!    `NUMBER_OF_PROCESSORS` and, when the processor is known, `PROCESSOR_IDENTIFIER`,
//!    `PROCESSOR_LEVEL` and `PROCESSOR_REVISION` ([`Processor`]).
//! 2. The system variables: `HKLM\SYSTEM\CurrentControlSet\Control\Session
//!    Manager\Environment` (Learn: "Environment Variables").
//! 3. For a user: the logon variables, `HKU\<sid>\Volatile Environment` (`USERPROFILE`,
//!    `HOMEDRIVE`, `HOMEPATH`, `APPDATA`, `LOCALAPPDATA`, `USERNAME`, `USERDOMAIN`,
//!    `LOGONSERVER`, ...), with the user's profile as fallback; `SESSIONNAME` (`Console`) when the
//!    token is in an interactive session (not session 0: the sheet's SSH logon has none, the
//!    interactive one has it).
//! 4. For a user: the user's variables, `HKU\<sid>\Environment` (Learn: "Environment
//!    Variables"). The user's `Path` is appended to the system's with a `;` rather than
//!    replacing it.
//!
//! A `REG_EXPAND_SZ` value is expanded against what is set so far; other value types than
//! `REG_SZ` and `REG_EXPAND_SZ` are skipped. Where a registry value is missing, the layout's
//! own value ([`Layout`]) stands in, so a process always gets every variable.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cmp::Ordering;
use poly_ntobj::name::upcase;

/// `REG_SZ`, `REG_EXPAND_SZ` (`winnt.h`).
pub const REG_SZ: u32 = 1;
pub const REG_EXPAND_SZ: u32 = 2;

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

/// Which master key a registry path is below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Root {
    /// `\REGISTRY\MACHINE` (`HKEY_LOCAL_MACHINE`).
    Machine,
    /// `\REGISTRY\USER` (`HKEY_USERS`).
    Users,
}

/// Where [`build`] reads the registry.
pub trait Source {
    /// The value `name` of the key `path` below `root`: its type and data (UTF-16LE bytes for a
    /// string), or `None` when the key or the value does not exist.
    fn value(&self, root: Root, path: &str, name: &str) -> Option<(u32, Vec<u8>)>;
    /// Every value of the key `path` below `root`, in the key's order: name, type, data. Empty
    /// when the key does not exist.
    fn values(&self, root: Root, path: &str) -> Vec<(String, u32, Vec<u8>)>;
}

/// A `REG_SZ` / `REG_EXPAND_SZ` value's text: UTF-16LE, up to the first NUL.
pub fn text_of(data: &[u8]) -> String {
    let units: Vec<u16> =
        data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    String::from_utf16_lossy(&units)
}

/// The string value `name` of the key at `path`: its type and text.
pub fn string_value(src: &dyn Source, root: Root, path: &str, name: &str) -> Option<(u32, String)> {
    let (kind, data) = src.value(root, path, name)?;
    matches!(kind, REG_SZ | REG_EXPAND_SZ).then(|| (kind, text_of(&data)))
}

/// `REG_DWORD` (`winnt.h`).
const REG_DWORD: u32 = 4;

/// `SYSTEM\ControlSet00n`, the control set `SYSTEM\Select`'s `Current` names (what the
/// `CurrentControlSet` link points at); `ControlSet001` when there is none.
pub fn current_control_set(src: &dyn Source) -> String {
    let current = src
        .value(Root::Machine, "SYSTEM\\Select", "Current")
        .filter(|(kind, data)| *kind == REG_DWORD && data.len() == 4)
        .map_or(1, |(_, d)| u32::from_le_bytes([d[0], d[1], d[2], d[3]]));
    alloc::format!("SYSTEM\\ControlSet{current:03}")
}

/// `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion`.
pub const CURRENT_VERSION: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion";
/// `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion`.
pub const NT_CURRENT_VERSION: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";
/// `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList`.
pub const PROFILE_LIST: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\ProfileList";

/// The folders the machine variables fall back to when the registry does not name them (ADR
/// 0073's layout).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub system_drive: &'static str,
    pub system_root: &'static str,
    pub program_files: &'static str,
    pub program_files_x86: &'static str,
    pub common_files: &'static str,
    pub common_files_x86: &'static str,
    pub program_data: &'static str,
    pub public: &'static str,
    pub computer_name: &'static str,
}

/// Poly's layout (ADR 0073; `poly_personality_nt::layout`'s constants, whose host tests check
/// that the two agree).
pub const POLY_LAYOUT: Layout = Layout {
    system_drive: "C:",
    system_root: "C:\\Windows",
    program_files: "C:\\Program Files",
    program_files_x86: "C:\\Program Files (x86)",
    common_files: "C:\\Program Files\\Common Files",
    common_files_x86: "C:\\Program Files (x86)\\Common Files",
    program_data: "C:\\ProgramData",
    public: "C:\\Users\\Public",
    computer_name: "POLY",
};

/// The user whose environment is built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserInfo {
    /// The account name (`%USERNAME%`).
    pub name: String,
    /// Its SID as a string: `HKEY_USERS\<sid>` is its hive.
    pub sid: String,
    /// Its profile folder (`%USERPROFILE%`), a DOS path with a drive.
    pub profile: String,
    /// The Windows session of its token (`TokenSessionId`): 0 is the services' session.
    pub session: u32,
}

impl UserInfo {
    /// `%HOMEPATH%`: the profile without its drive.
    pub fn home_path(&self) -> String {
        match self.profile.get(1..2) {
            Some(":") => self.profile[2..].to_string(),
            _ => self.profile.clone(),
        }
    }

    /// `%HOMEDRIVE%`: the profile's drive.
    pub fn home_drive(&self) -> String {
        match self.profile.get(1..2) {
            Some(":") => self.profile[..2].to_string(),
            _ => String::new(),
        }
    }

    pub fn app_data(&self) -> String {
        alloc::format!("{}\\AppData\\Roaming", self.profile)
    }

    pub fn local_app_data(&self) -> String {
        alloc::format!("{}\\AppData\\Local", self.profile)
    }
}

/// What `CPUID` says about the processor (Intel SDM Vol. 2A, "CPUID": the vendor string of leaf
/// 0, the display family, model and stepping of leaf 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Processor {
    pub vendor: [u8; 12],
    pub family: u32,
    pub model: u32,
    pub stepping: u32,
}

impl Processor {
    /// The display family, model and stepping of leaf 1's `EAX` (SDM: "Version Information
    /// Returned by CPUID in EAX": the extended family is added for family 0xF, the extended
    /// model is the high nibble for families 6 and 0xF).
    pub fn from_cpuid(vendor: [u8; 12], eax1: u32) -> Processor {
        let base_family = (eax1 >> 8) & 0xf;
        let family = if base_family == 0xf { base_family + ((eax1 >> 20) & 0xff) } else { base_family };
        let mut model = (eax1 >> 4) & 0xf;
        if base_family == 0x6 || base_family == 0xf {
            model |= ((eax1 >> 16) & 0xf) << 4;
        }
        Processor { vendor, family, model, stepping: eax1 & 0xf }
    }

    fn vendor_str(&self) -> &str {
        core::str::from_utf8(&self.vendor).unwrap_or("").trim_end_matches('\0')
    }

    /// `%PROCESSOR_IDENTIFIER%`: the architecture, family, model, stepping and vendor
    /// (`Intel64` for Intel's 64-bit processors, `AMD64` for the others).
    pub fn identifier(&self) -> String {
        let arch = if self.vendor_str() == "GenuineIntel" { "Intel64" } else { "AMD64" };
        alloc::format!(
            "{arch} Family {} Model {} Stepping {}, {}",
            self.family,
            self.model,
            self.stepping,
            self.vendor_str()
        )
    }

    /// `%PROCESSOR_LEVEL%`: the family (Learn: `SYSTEM_INFO.wProcessorLevel`).
    pub fn level(&self) -> String {
        alloc::format!("{}", self.family)
    }

    /// `%PROCESSOR_REVISION%`: `xxyy`, the model and the stepping in hexadecimal (Learn:
    /// `SYSTEM_INFO.wProcessorRevision`).
    pub fn revision(&self) -> String {
        alloc::format!("{:02x}{:02x}", self.model & 0xff, self.stepping & 0xff)
    }
}

/// What the caller knows about the machine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Machine {
    /// `NUMBER_OF_PROCESSORS`.
    pub processors: u32,
    /// The `PROCESSOR_*` variables' source, if known.
    pub processor: Option<Processor>,
}

/// `SESSIONNAME` of a token in an interactive session (Learn: "Recognized environment
/// variables": "the name of the session", the console's for a local logon).
pub const CONSOLE_SESSION_NAME: &str = "Console";

/// `STATUS_OBJECT_TYPE_MISMATCH` (`ntstatus.h`).
const STATUS_OBJECT_TYPE_MISMATCH: u32 = 0xC000_0024;

/// What a failed query of `CreateEnvironmentBlock`'s token means (the sheet's `token_kinds` and
/// `bad_handles`): `None` when it succeeded; `Some(Ok(()))` when the handle is no token (the
/// process's handle, the pseudo handle -1), which builds the block as for a NULL token;
/// `Some(Err(error))` otherwise (a value that is no handle: `ERROR_INVALID_HANDLE`; a token
/// opened without `TOKEN_QUERY`: `ERROR_ACCESS_DENIED`).
pub fn token_query_failure(status: u32) -> Option<Result<(), u32>> {
    match status {
        0 => None,
        STATUS_OBJECT_TYPE_MISMATCH => Some(Ok(())),
        s => Some(Err(crate::error::from_status(crate::NtStatus(s)))),
    }
}

/// Every `REG_SZ` / `REG_EXPAND_SZ` value of a key, set into `b` (expanded against `b` as it
/// is). The `Path` of `append_path` is joined to an existing `Path` with `;`.
fn apply_key(b: &mut Block, src: &dyn Source, root: Root, path: &str, append_path: bool) {
    for (name, kind, data) in src.values(root, path) {
        if !matches!(kind, REG_SZ | REG_EXPAND_SZ) || name.is_empty() {
            continue;
        }
        let raw = text_of(&data);
        let value = if kind == REG_EXPAND_SZ { b.expand(&raw) } else { raw };
        if append_path
            && same_name(&name, "Path")
            && let Some(old) = b.get("Path").filter(|p| !p.is_empty())
        {
            let joined =
                if old.ends_with(';') { alloc::format!("{old}{value}") } else { alloc::format!("{old};{value}") };
            b.set(&name, &joined);
            continue;
        }
        b.set(&name, &value);
    }
}

/// The default environment of a new process of `user` (`None`: the system's, as for a NULL
/// token), starting from `base` (see the module documentation for the order).
pub fn build(src: &dyn Source, layout: &Layout, user: Option<&UserInfo>, machine: Machine, base: Block) -> Block {
    let mut b = base;
    let machine_value = |b: &mut Block, var: &str, path: &str, name: &str, fallback: &str| {
        let value = match string_value(src, Root::Machine, path, name) {
            Some((REG_EXPAND_SZ, raw)) => b.expand(&raw),
            Some((_, raw)) => raw,
            None => fallback.into(),
        };
        b.set(var, &value);
    };
    // 1. The machine's variables.
    b.set("SystemDrive", layout.system_drive);
    machine_value(&mut b, "SystemRoot", NT_CURRENT_VERSION, "SystemRoot", layout.system_root);
    machine_value(&mut b, "ProgramFiles", CURRENT_VERSION, "ProgramFilesDir", layout.program_files);
    machine_value(&mut b, "ProgramW6432", CURRENT_VERSION, "ProgramW6432Dir", layout.program_files);
    machine_value(&mut b, "ProgramFiles(x86)", CURRENT_VERSION, "ProgramFilesDir (x86)", layout.program_files_x86);
    machine_value(&mut b, "CommonProgramFiles", CURRENT_VERSION, "CommonFilesDir", layout.common_files);
    machine_value(&mut b, "CommonProgramW6432", CURRENT_VERSION, "CommonW6432Dir", layout.common_files);
    machine_value(&mut b, "CommonProgramFiles(x86)", CURRENT_VERSION, "CommonFilesDir (x86)", layout.common_files_x86);
    machine_value(&mut b, "ProgramData", PROFILE_LIST, "ProgramData", layout.program_data);
    let program_data = b.get("ProgramData").unwrap_or(layout.program_data).to_string();
    b.set("ALLUSERSPROFILE", &program_data);
    machine_value(&mut b, "PUBLIC", PROFILE_LIST, "Public", layout.public);
    let control = current_control_set(src);
    let computer = alloc::format!("{control}\\Control\\ComputerName\\ComputerName");
    machine_value(&mut b, "COMPUTERNAME", &computer, "ComputerName", layout.computer_name);
    b.set("NUMBER_OF_PROCESSORS", &alloc::format!("{}", machine.processors.max(1)));
    if let Some(p) = machine.processor {
        b.set("PROCESSOR_IDENTIFIER", &p.identifier());
        b.set("PROCESSOR_LEVEL", &p.level());
        b.set("PROCESSOR_REVISION", &p.revision());
    }

    // 2. The system variables.
    apply_key(&mut b, src, Root::Machine, &alloc::format!("{control}\\Control\\Session Manager\\Environment"), false);

    let Some(user) = user else { return b };
    // 3. The user's logon variables, with the profile's as fallbacks.
    for (var, fallback) in [
        ("USERPROFILE", user.profile.clone()),
        ("HOMEDRIVE", user.home_drive()),
        ("HOMEPATH", user.home_path()),
        ("APPDATA", user.app_data()),
        ("LOCALAPPDATA", user.local_app_data()),
        ("USERNAME", user.name.clone()),
    ] {
        b.set(var, &fallback);
    }
    apply_key(&mut b, src, Root::Users, &alloc::format!("{}\\Volatile Environment", user.sid), false);
    if user.session != 0 {
        b.set("SESSIONNAME", CONSOLE_SESSION_NAME);
    }

    // 4. The user's variables.
    apply_key(&mut b, src, Root::Users, &alloc::format!("{}\\Environment", user.sid), true);
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// A registry made of `(root, key, name, type, text)` rows.
    struct Rows(Vec<(Root, &'static str, &'static str, u32, &'static str)>);

    fn sz(s: &str) -> Vec<u8> {
        s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
    }

    impl Source for Rows {
        fn value(&self, root: Root, path: &str, name: &str) -> Option<(u32, Vec<u8>)> {
            self.0.iter().find(|r| r.0 == root && r.1.eq_ignore_ascii_case(path) && r.2.eq_ignore_ascii_case(name)).map(
                |r| (r.3, if r.3 == REG_DWORD { r.4.parse::<u32>().unwrap().to_le_bytes().to_vec() } else { sz(r.4) }),
            )
        }
        fn values(&self, root: Root, path: &str) -> Vec<(String, u32, Vec<u8>)> {
            self.0
                .iter()
                .filter(|r| r.0 == root && r.1.eq_ignore_ascii_case(path))
                .map(|r| (r.2.to_string(), r.3, sz(r.4)))
                .collect()
        }
    }

    const SID: &str = "S-1-5-21-1-2-3-1002";
    const ENV: &str = "SYSTEM\\ControlSet001\\Control\\Session Manager\\Environment";

    fn machine() -> Rows {
        Rows(vec![
            (Root::Machine, ENV, "Path", REG_EXPAND_SZ, "%SystemRoot%\\system32;%SystemRoot%"),
            (Root::Machine, ENV, "TEMP", REG_EXPAND_SZ, "%SystemRoot%\\TEMP"),
            (Root::Machine, ENV, "OS", REG_SZ, "Windows_NT"),
            (Root::Machine, ENV, "Count", REG_DWORD, "7"),
            (Root::Users, "S-1-5-21-1-2-3-1002\\Volatile Environment", "USERDOMAIN", REG_SZ, "POLY"),
            (
                Root::Users,
                "S-1-5-21-1-2-3-1002\\Environment",
                "TEMP",
                REG_EXPAND_SZ,
                "%USERPROFILE%\\AppData\\Local\\Temp",
            ),
            (Root::Users, "S-1-5-21-1-2-3-1002\\Environment", "Path", REG_EXPAND_SZ, "%USERPROFILE%\\bin"),
        ])
    }

    fn user(session: u32) -> UserInfo {
        UserInfo { name: "poly".into(), sid: SID.into(), profile: "C:\\Users\\poly".into(), session }
    }

    #[test]
    fn a_user_gets_the_machine_system_and_own_variables() {
        let b =
            build(&machine(), &POLY_LAYOUT, Some(&user(0)), Machine { processors: 3, processor: None }, Block::new());
        assert_eq!(b.get("SystemRoot"), Some("C:\\Windows"));
        assert_eq!(b.get("ALLUSERSPROFILE"), Some("C:\\ProgramData"));
        assert_eq!(b.get("NUMBER_OF_PROCESSORS"), Some("3"));
        assert_eq!(b.get("USERPROFILE"), Some("C:\\Users\\poly"));
        assert_eq!(b.get("HOMEDRIVE"), Some("C:"));
        assert_eq!(b.get("HOMEPATH"), Some("\\Users\\poly"));
        assert_eq!(b.get("APPDATA"), Some("C:\\Users\\poly\\AppData\\Roaming"));
        assert_eq!(b.get("USERDOMAIN"), Some("POLY"));
        assert_eq!(b.get("temp"), Some("C:\\Users\\poly\\AppData\\Local\\Temp"));
        assert_eq!(b.get("Path"), Some("C:\\Windows\\system32;C:\\Windows;C:\\Users\\poly\\bin"));
        assert_eq!(b.get("Count"), None, "non-string values are skipped");
        assert_eq!(b.get("SESSIONNAME"), None, "session 0 has no session name");
        assert_eq!(b.get("PROCESSOR_IDENTIFIER"), None);
    }

    #[test]
    fn an_interactive_session_has_a_session_name() {
        let b = build(&machine(), &POLY_LAYOUT, Some(&user(1)), Machine::default(), Block::new());
        assert_eq!(b.get("SESSIONNAME"), Some("Console"));
        assert_eq!(b.get("NUMBER_OF_PROCESSORS"), Some("1"));
    }

    #[test]
    fn no_user_is_the_system_part_only() {
        let b = build(&machine(), &POLY_LAYOUT, None, Machine::default(), Block::new());
        assert_eq!(b.get("TEMP"), Some("C:\\Windows\\TEMP"));
        assert_eq!(b.get("USERPROFILE"), None);
        assert_eq!(b.get("Path"), Some("C:\\Windows\\system32;C:\\Windows"));
    }

    #[test]
    fn the_base_block_is_kept_but_the_defaults_win() {
        let mut base = Block::new();
        base.set("=C:", "C:\\work");
        base.set("MINE", "1");
        base.set("TEMP", "C:\\scratch");
        base.set("PATH", "C:\\tools");
        let b = build(&machine(), &POLY_LAYOUT, Some(&user(0)), Machine::default(), base);
        assert_eq!(b.get("=C:"), Some("C:\\work"));
        assert_eq!(b.get("MINE"), Some("1"));
        assert_eq!(b.get("TEMP"), Some("C:\\Users\\poly\\AppData\\Local\\Temp"));
        // The system's Path replaces the caller's; the user's is appended to it.
        assert_eq!(b.get("Path"), Some("C:\\Windows\\system32;C:\\Windows;C:\\Users\\poly\\bin"));
        let text = String::from_utf16(&b.to_utf16()).unwrap();
        assert!(text.starts_with("=C:=C:\\work\0"), "{text}");
    }

    #[test]
    fn the_processor_variables_follow_cpuid() {
        // Family 6, model 0x55 (extended model 5, model 5), stepping 7.
        let p = Processor::from_cpuid(*b"GenuineIntel", 0x0005_0657);
        assert_eq!((p.family, p.model, p.stepping), (6, 0x55, 7));
        assert_eq!(p.identifier(), "Intel64 Family 6 Model 85 Stepping 7, GenuineIntel");
        assert_eq!(p.level(), "6");
        assert_eq!(p.revision(), "5507");
        // Family 0xF with extended family 8 (family 23), model 1 / extended 3, stepping 0.
        let a = Processor::from_cpuid(*b"AuthenticAMD", 0x0083_0F10);
        assert_eq!(a.identifier(), "AMD64 Family 23 Model 49 Stepping 0, AuthenticAMD");
        let b = build(&machine(), &POLY_LAYOUT, None, Machine { processors: 2, processor: Some(a) }, Block::new());
        assert_eq!(b.get("PROCESSOR_LEVEL"), Some("23"));
        assert_eq!(b.get("PROCESSOR_REVISION"), Some("3100"));
    }

    #[test]
    fn token_failures_end_the_call_or_fall_back() {
        assert_eq!(token_query_failure(0), None);
        assert_eq!(token_query_failure(0xC000_0024), Some(Ok(())), "not a token: as a NULL token");
        assert_eq!(token_query_failure(0xC000_0008), Some(Err(6)), "not a handle");
        assert_eq!(token_query_failure(0xC000_0022), Some(Err(5)), "no TOKEN_QUERY");
    }

    #[test]
    fn a_missing_registry_gives_the_layout() {
        let b = build(&Rows(vec![]), &POLY_LAYOUT, Some(&user(0)), Machine::default(), Block::new());
        assert_eq!(b.get("ProgramFiles"), Some("C:\\Program Files"));
        assert_eq!(b.get("COMPUTERNAME"), Some("POLY"));
        assert_eq!(b.get("PUBLIC"), Some("C:\\Users\\Public"));
        assert_eq!(b.get("Path"), None);
    }
}
