//! The default system hives: a minimal set of keys and values Windows
//! programs commonly read, made the first time Poly boots without hive
//! files (and kept in memory when there is no disk to save them to).
//!
//! Every key and value here is listed with where its meaning is documented
//! in ADR 0066's tables. The values are **Poly's own**: none was copied from
//! a Windows installation. Where Learn documents only a name (or nothing),
//! the value is what that name means for Poly, and the ADR says so.
//!
//! | Hive | Mounted at | File |
//! |---|---|---|
//! | system | `\REGISTRY\MACHINE\SYSTEM` (`HKLM\SYSTEM`) | `SYSTEM` |
//! | software | `\REGISTRY\MACHINE\SOFTWARE` (`HKLM\SOFTWARE`) | `SOFTWARE` |
//! | default user | `\REGISTRY\USER\.DEFAULT` (`HKU\.DEFAULT`) | `DEFAULT` |
//! | Poly's user | `\REGISTRY\USER\S-1-5-21-0-0-0-1000` (`HKCU`) | `USER` |
//!
//! Volatile keys are made at every boot ([`volatile_keys`]):
//! `HKLM\HARDWARE`, the `HKLM\SYSTEM\CurrentControlSet` symbolic link to
//! `ControlSet001`, and the user's `Volatile Environment`.

use crate::consts::*;
use crate::hive;
use crate::tree::{CreateOptions, KeyId, Registry};
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::name::wide;

/// Poly's single user's security identifier: a local account SID
/// (`S-1-5-21-…`, Learn: "Well-known SIDs") with three zero sub-authorities
/// and relative id 1000. Poly's own choice until tokens exist (the
/// `advapi32` milestone); `HKEY_CURRENT_USER` is `\REGISTRY\USER\<this>`.
pub const POLY_USER_SID: &str = "S-1-5-21-0-0-0-1000";
/// The user's name and the machine's (Poly's own values).
pub const POLY_USER_NAME: &str = "poly";
pub const POLY_COMPUTER_NAME: &str = "POLY";

/// A default value's data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Data {
    Sz(&'static str),
    ExpandSz(&'static str),
    Dword(u32),
    MultiSz(&'static [&'static str]),
}

impl Data {
    pub fn kind(self) -> u32 {
        match self {
            Data::Sz(_) => REG_SZ,
            Data::ExpandSz(_) => REG_EXPAND_SZ,
            Data::Dword(_) => REG_DWORD,
            Data::MultiSz(_) => REG_MULTI_SZ,
        }
    }

    /// The bytes as stored: strings UTF-16LE with their terminating NUL
    /// (Learn: "ZwSetValueKey": "If Type is REG_XXX_SZ, this value must
    /// include space for any terminating zeros"), a multi-string with one
    /// more NUL.
    pub fn bytes(self) -> Vec<u8> {
        match self {
            Data::Sz(s) | Data::ExpandSz(s) => sz(s),
            Data::Dword(v) => v.to_le_bytes().to_vec(),
            Data::MultiSz(list) => {
                let mut out = Vec::new();
                for s in list {
                    out.extend(sz(s));
                }
                out.extend_from_slice(&[0, 0]);
                out
            }
        }
    }
}

/// A string as `REG_SZ` data: UTF-16LE and a NUL.
pub fn sz(s: &str) -> Vec<u8> {
    s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}

/// A default key and its values. Its path is below the hive root.
#[derive(Clone, Copy, Debug)]
pub struct DefaultKey {
    pub path: &'static str,
    pub values: &'static [(&'static str, Data)],
}

/// Which master key a hive is mounted under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Master {
    Machine,
    User,
}

/// A default hive.
#[derive(Clone, Copy, Debug)]
pub struct DefaultHive {
    pub master: Master,
    /// The mount point's name below the master key.
    pub name: &'static str,
    /// The file it is saved under.
    pub file: &'static str,
    pub keys: &'static [DefaultKey],
}

use Data::*;

const fn k(path: &'static str, values: &'static [(&'static str, Data)]) -> DefaultKey {
    DefaultKey { path, values }
}

/// The version Poly reports: Windows 10 (`CurrentMajorVersionNumber` 10),
/// build 19045, the last Windows 10 build (Learn: "Windows 10 release
/// information"). Programs compare build numbers to decide which APIs they
/// may use, so this states the API level Poly aims at; `RtlGetVersion`
/// and the PEB report the same numbers when they exist.
pub const BUILD: &str = "19045";

pub const SYSTEM_KEYS: &[DefaultKey] = &[
    k("Select", &[("Current", Dword(1)), ("Default", Dword(1)), ("Failed", Dword(0)), ("LastKnownGood", Dword(1))]),
    k("ControlSet001\\Control\\ComputerName\\ComputerName", &[("ComputerName", Sz(POLY_COMPUTER_NAME))]),
    k("ControlSet001\\Control\\ProductOptions", &[("ProductType", Sz("WinNT"))]),
    k(
        "ControlSet001\\Control\\Session Manager\\Environment",
        &[
            ("ComSpec", ExpandSz("%SystemRoot%\\system32\\cmd.exe")),
            ("OS", Sz("Windows_NT")),
            ("Path", ExpandSz("%SystemRoot%\\system32;%SystemRoot%")),
            ("PATHEXT", Sz(".COM;.EXE;.BAT;.CMD")),
            ("PROCESSOR_ARCHITECTURE", Sz("AMD64")),
            ("TEMP", ExpandSz("%SystemRoot%\\TEMP")),
            ("TMP", ExpandSz("%SystemRoot%\\TEMP")),
            ("windir", ExpandSz("%SystemRoot%")),
        ],
    ),
    k("ControlSet001\\Control\\Nls\\CodePage", &[("ACP", Sz("1252")), ("OEMCP", Sz("437"))]),
    k("ControlSet001\\Control\\TimeZoneInformation", &[("TimeZoneKeyName", Sz("UTC")), ("Bias", Dword(0))]),
    k("ControlSet001\\Enum", &[]),
    k("ControlSet001\\Hardware Profiles\\Current", &[]),
    k("ControlSet001\\Services", &[]),
];

pub const SOFTWARE_KEYS: &[DefaultKey] = &[
    k("Classes", &[]),
    k(
        "Microsoft\\Windows NT\\CurrentVersion",
        &[
            ("ProductName", Sz("Poly")),
            ("CurrentMajorVersionNumber", Dword(10)),
            ("CurrentMinorVersionNumber", Dword(0)),
            ("CurrentVersion", Sz("6.3")),
            ("CurrentBuild", Sz(BUILD)),
            ("CurrentBuildNumber", Sz(BUILD)),
            ("InstallationType", Sz("Client")),
            ("SystemRoot", Sz("C:\\Windows")),
            ("RegisteredOwner", Sz(POLY_USER_NAME)),
            ("RegisteredOrganization", Sz("")),
        ],
    ),
    k(
        "Microsoft\\Windows\\CurrentVersion",
        &[
            ("ProgramFilesDir", Sz("C:\\Program Files")),
            ("ProgramFilesDir (x86)", Sz("C:\\Program Files (x86)")),
            ("ProgramW6432Dir", Sz("C:\\Program Files")),
            ("CommonFilesDir", Sz("C:\\Program Files\\Common Files")),
        ],
    ),
    k("Microsoft\\Windows\\CurrentVersion\\App Paths", &[]),
    k("Microsoft\\Windows\\CurrentVersion\\Run", &[]),
    k("Microsoft\\Windows\\CurrentVersion\\Uninstall", &[]),
];

const USER_TEMP: Data = ExpandSz("%USERPROFILE%\\AppData\\Local\\Temp");

pub const DEFAULT_USER_KEYS: &[DefaultKey] = &[
    k("Environment", &[("TEMP", USER_TEMP), ("TMP", USER_TEMP)]),
    k("Control Panel\\International", &[("LocaleName", Sz("en-US"))]),
    k("Software\\Classes", &[]),
];

pub const USER_KEYS: &[DefaultKey] = &[
    k("Environment", &[("TEMP", USER_TEMP), ("TMP", USER_TEMP)]),
    k("Control Panel\\International", &[("LocaleName", Sz("en-US"))]),
    k("Console", &[]),
    k("Software\\Classes", &[]),
    k("Software\\Microsoft\\Windows\\CurrentVersion\\Run", &[]),
];

/// The user's `Volatile Environment` (Learn: "Recognized environment
/// variables" names each).
pub const VOLATILE_ENVIRONMENT: &[(&str, Data)] = &[
    ("APPDATA", Sz("C:\\Users\\poly\\AppData\\Roaming")),
    ("HOMEDRIVE", Sz("C:")),
    ("HOMEPATH", Sz("\\Users\\poly")),
    ("LOCALAPPDATA", Sz("C:\\Users\\poly\\AppData\\Local")),
    ("LOGONSERVER", Sz("\\\\POLY")),
    ("USERDOMAIN", Sz(POLY_COMPUTER_NAME)),
    ("USERNAME", Sz(POLY_USER_NAME)),
    ("USERPROFILE", Sz("C:\\Users\\poly")),
];

/// The four default hives.
pub const HIVES: [DefaultHive; 4] = [
    DefaultHive { master: Master::Machine, name: "SYSTEM", file: "SYSTEM", keys: SYSTEM_KEYS },
    DefaultHive { master: Master::Machine, name: "SOFTWARE", file: "SOFTWARE", keys: SOFTWARE_KEYS },
    DefaultHive { master: Master::User, name: ".DEFAULT", file: "DEFAULT", keys: DEFAULT_USER_KEYS },
    DefaultHive { master: Master::User, name: POLY_USER_SID, file: "USER", keys: USER_KEYS },
];

/// How a default hive came to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Read from its file.
    Loaded,
    /// No file: made from the defaults (dirty until saved).
    Created,
    /// The file was not a valid hive file ([`hive::decode`]): made from the
    /// defaults instead; the next save replaces the file.
    Corrupt(NtStatus),
}

/// Create (or open) each component of `path` below `parent`, non-volatile
/// unless `volatile`.
pub fn make_path(reg: &mut Registry, parent: KeyId, path: &str, volatile: bool, now: u64) -> Result<KeyId, NtStatus> {
    let mut cur = parent;
    for c in path.split('\\').filter(|c| !c.is_empty()) {
        let opts = CreateOptions { volatile, ..CreateOptions::default() };
        cur = reg.create_key(cur, &wide(c), opts, now)?.0;
    }
    Ok(cur)
}

fn set(reg: &mut Registry, key: KeyId, values: &[(&str, Data)], now: u64) -> Result<(), NtStatus> {
    for (name, data) in values {
        reg.set_value(key, &wide(name), data.kind(), &data.bytes(), now)?;
    }
    Ok(())
}

/// Fill a freshly mounted, empty hive with its default keys. The software
/// hive's `InstallDate` (`REG_DWORD`, seconds since 1970) is the time of
/// that first boot.
pub fn populate(reg: &mut Registry, root: KeyId, spec: &DefaultHive, now: u64) -> Result<(), NtStatus> {
    for key in spec.keys {
        let id = make_path(reg, root, key.path, false, now)?;
        set(reg, id, key.values, now)?;
        if spec.file == "SOFTWARE" && key.path == "Microsoft\\Windows NT\\CurrentVersion" {
            let unix = now.saturating_sub(FILETIME_UNIX_EPOCH) / 10_000_000;
            reg.set_value(id, &wide("InstallDate"), REG_DWORD, &(unix as u32).to_le_bytes(), now)?;
        }
    }
    Ok(())
}

/// Mount the default hives, each from the file `load` returns for its name
/// (or from the defaults when there is none, or it is corrupt), then make
/// the volatile keys. Returns what happened to each hive, in [`HIVES`]
/// order.
pub fn install(
    reg: &mut Registry,
    now: u64,
    load: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
) -> Result<Vec<Outcome>, NtStatus> {
    let mut out = Vec::new();
    for spec in &HIVES {
        let parent = match spec.master {
            Master::Machine => reg.machine(),
            Master::User => reg.user(),
        };
        let file = load(spec.file).map(|b| hive::decode(&b));
        let outcome = match &file {
            Some(Ok(_)) => Outcome::Loaded,
            Some(Err(e)) => Outcome::Corrupt(*e),
            None => Outcome::Created,
        };
        let loaded = file.and_then(Result::ok);
        let id = reg.mount_hive(parent, &wide(spec.name), spec.file, loaded.as_ref(), now)?;
        if loaded.is_none() {
            let root = reg.hive(id).root;
            populate(reg, root, spec, now)?;
        }
        out.push(outcome);
    }
    volatile_keys(reg, now)?;
    Ok(out)
}

/// The keys made at every boot and never saved:
///
/// - `HKLM\HARDWARE\DESCRIPTION\System` and `HKLM\HARDWARE\DEVICEMAP`
///   (Learn: "ZwSetValueKey": the system writes the hardware description
///   tree; empty until device detection fills them).
/// - `HKLM\SYSTEM\CurrentControlSet`, a symbolic link to
///   `\REGISTRY\MACHINE\SYSTEM\ControlSet001` (the control set `Select`'s
///   `Current` names).
/// - `HKU\<user>\Volatile Environment` with [`VOLATILE_ENVIRONMENT`].
pub fn volatile_keys(reg: &mut Registry, now: u64) -> Result<(), NtStatus> {
    let machine = reg.machine();
    make_path(reg, machine, "HARDWARE\\DESCRIPTION\\System", true, now)?;
    make_path(reg, machine, "HARDWARE\\DEVICEMAP", true, now)?;
    if let Some(system) = reg.find(machine, "SYSTEM")
        && reg.child(system, &wide("CurrentControlSet")).is_none()
    {
        let opts = CreateOptions { volatile: true, link: true, class: &[] };
        let (link, _) = reg.create_key(system, &wide("CurrentControlSet"), opts, now)?;
        let target: Vec<u8> =
            "\\REGISTRY\\MACHINE\\SYSTEM\\ControlSet001".encode_utf16().flat_map(u16::to_le_bytes).collect();
        reg.set_value(link, &wide(SYMBOLIC_LINK_VALUE), REG_LINK, &target, now)?;
    }
    let user = reg.user();
    if let Some(me) = reg.find(user, POLY_USER_SID) {
        let env = make_path(reg, me, "Volatile Environment", true, now)?;
        set(reg, env, VOLATILE_ENVIRONMENT, now)?;
    }
    Ok(())
}
