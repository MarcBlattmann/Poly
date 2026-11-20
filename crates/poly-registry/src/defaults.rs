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
//! | Poly's user | `\REGISTRY\USER\S-1-5-21-520193847-884726105-1936024571-1002` (`HKCU`) | `USER` |
//!
//! Volatile keys are made at every boot ([`volatile_keys`]):
//! `HKLM\HARDWARE`, the `HKLM\SYSTEM\CurrentControlSet` symbolic link to
//! `ControlSet001`, the user's `Volatile Environment`, and `HKU\<user>_Classes`,
//! a symbolic link to the user's `Software\Classes`.
//!
//! The values that name folders (`ProgramFilesDir`, `SystemRoot`, the
//! environment's paths, the user's profile) must agree with the Windows
//! file system layout of ADR 0073 (`poly_personality_nt::layout`), whose
//! sync adds the rest (`ProfileList`, `Shell Folders`) at every boot and
//! whose host tests check that these agree.

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
pub const POLY_USER_SID: &str = "S-1-5-21-520193847-884726105-1936024571-1002";
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

pub(crate) const fn k(path: &'static str, values: &'static [(&'static str, Data)]) -> DefaultKey {
    DefaultKey { path, values }
}

/// The version Poly reports: Windows 11 (`CurrentMajorVersionNumber` 10, as on Windows 11),
/// build 26200 (25H2, Learn: "Windows 11 release information"), the build of the machine the
/// spec sheets were observed on (ADR 0189). Programs compare build numbers to decide which APIs
/// they may use, so this states the API level Poly aims at; `RtlGetVersion` and the PEB report
/// the same numbers when they exist.
pub const BUILD: &str = "26200";

/// System variables (`Session Manager\Environment`, `REG_EXPAND_SZ`) that every process has
/// (`specs/windows/userenv.dll/CreateEnvironmentBlock.yaml` `variables_present`), with Poly's
/// values: PowerShell's module folders (Learn: "about_PSModulePath") and the drivers' data folder
/// (Learn: "DriverData"). The layout sync adds them to a hive made before they were here.
pub const SYSTEM_ENVIRONMENT_EXTRA: [(&str, &str); 2] = [
    (
        "PSModulePath",
        "%ProgramFiles%\\WindowsPowerShell\\Modules;%SystemRoot%\\system32\\WindowsPowerShell\\v1.0\\Modules",
    ),
    ("DriverData", "%SystemRoot%\\System32\\Drivers\\DriverData"),
];

pub const SYSTEM_KEYS: &[DefaultKey] = &[
    k("Select", &[("Current", Dword(1)), ("Default", Dword(1)), ("Failed", Dword(0)), ("LastKnownGood", Dword(1))]),
    k("ControlSet001\\Control\\ComputerName\\ComputerName", &[("ComputerName", Sz(POLY_COMPUTER_NAME))]),
    k("ControlSet001\\Control\\ProductOptions", &[("ProductType", Sz("WinNT"))]),
    k(
        "ControlSet001\\Control\\Session Manager\\Environment",
        &[
            ("ComSpec", ExpandSz("%SystemRoot%\\system32\\cmd.exe")),
            ("OS", Sz("Windows_NT")),
            // The system folders, then those of the command-line tools of WMI and PowerShell
            // (Learn: "WMIC", "about_PSModulePath"); CreateEnvironmentBlock.yaml: the Path has
            // more after `%SystemRoot%;`.
            (
                "Path",
                ExpandSz(
                    "%SystemRoot%\\system32;%SystemRoot%;%SystemRoot%\\System32\\Wbem;%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\",
                ),
            ),
            ("PATHEXT", Sz(".COM;.EXE;.BAT;.CMD")),
            ("PROCESSOR_ARCHITECTURE", Sz("AMD64")),
            ("TEMP", ExpandSz("%SystemRoot%\\TEMP")),
            ("TMP", ExpandSz("%SystemRoot%\\TEMP")),
            ("windir", ExpandSz("%SystemRoot%")),
            ("PSModulePath", ExpandSz(SYSTEM_ENVIRONMENT_EXTRA[0].1)),
            ("DriverData", ExpandSz(SYSTEM_ENVIRONMENT_EXTRA[1].1)),
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
    // The class-registration roots a Windows machine has below `Classes` in both views
    // (`RegistryDefaults.ClassesRoots.yaml` `names`). The standard COM registrations below
    // `CLSID`, `Interface` and `TypeLib` are [`crate::comdefaults`]'s, seeded per view by
    // [`wow64_view_keys`].
    k("Classes\\CLSID", &[]),
    k("Classes\\Interface", &[]),
    k("Classes\\TypeLib", &[]),
    k("Classes\\AppID", &[]),
    k("Classes\\Installer", &[]),
    k("Classes\\MIME", &[]),
    k("Classes\\Local Settings", &[]),
    k("Classes\\Media Type", &[]),
    k("Classes\\MediaFoundation", &[]),
    k("Classes\\DirectShow", &[]),
    k("Classes\\Drive", &[]),
    k("Classes\\Directory", &[]),
    k("Classes\\Folder", &[]),
    k("Classes\\*", &[]),
    k("Classes\\exefile", &[]),
    k("Classes\\dllfile", &[]),
    k("Classes\\.exe", &[]),
    k("Classes\\.dll", &[]),
    k("Classes\\.tlb", &[]),
    k("Classes\\.lnk", &[]),
    k("Classes\\http", &[]),
    k("Classes\\https", &[]),
    k("Classes\\SystemFileAssociations", &[]),
    // `RegistryDefaults.Wow6432Node.yaml` `software_64`, `microsoft`.
    k("Clients", &[]),
    k("Policies", &[]),
    k("RegisteredApplications", &[]),
    k("Microsoft\\Ole", &[]),
    k("Microsoft\\COM3", &[]),
    k("Microsoft\\Rpc", &[]),
    k(
        "Microsoft\\Windows NT\\CurrentVersion",
        &[
            ("ProductName", Sz("Poly")),
            ("CurrentMajorVersionNumber", Dword(10)),
            ("CurrentMinorVersionNumber", Dword(0)),
            ("CurrentVersion", Sz("6.3")),
            ("CurrentBuild", Sz(BUILD)),
            ("CurrentBuildNumber", Sz(BUILD)),
            // The release the build belongs to (Windows 11 25H2, `26200.9974`): programs
            // that report the OS version read these (Chrome's browser process, on Windows,
            // queries `UBR`, `DisplayVersion` and `ReleaseId`, the traces of ADR 0155).
            ("UBR", Dword(9974)),
            ("DisplayVersion", Sz("25H2")),
            ("ReleaseId", Sz("2009")),
            ("InstallationType", Sz("Client")),
            ("SystemRoot", Sz("C:\\Windows")),
            ("RegisteredOwner", Sz(POLY_USER_NAME)),
            ("RegisteredOrganization", Sz("")),
            // `RegistryDefaults.CurrentVersion.yaml` `nt_values`: the edition of the oracle (Home,
            // "Core"), `PathName` equal to `SystemRoot`, and the build lab strings, which are
            // machine data (Poly's own).
            ("EditionID", Sz("Core")),
            ("CompositionEditionID", Sz("Core")),
            ("PathName", Sz("C:\\Windows")),
            ("BuildLab", Sz("26200.poly")),
            ("BuildLabEx", Sz("26200.9974.poly")),
        ],
    ),
    k("Microsoft\\Windows\\CurrentVersion", CURRENT_VERSION_64),
    k("Microsoft\\Windows\\CurrentVersion\\Installer", INSTALLER_64),
    k("Microsoft\\Windows\\CurrentVersion\\App Paths", &[]),
    k("Microsoft\\Windows\\CurrentVersion\\Run", &[]),
    k("Microsoft\\Windows\\CurrentVersion\\RunOnce", &[]),
    k("Microsoft\\Windows\\CurrentVersion\\Uninstall", &[]),
    k("Microsoft\\Windows\\CurrentVersion\\Policies", &[]),
    k("Microsoft\\Windows\\CurrentVersion\\Explorer", &[]),
    k("Microsoft\\Windows\\CurrentVersion\\Setup", &[]),
    k("Microsoft\\Windows\\CurrentVersion\\Internet Settings", &[]),
];

/// `Microsoft\Windows\CurrentVersion`'s folders, as the 64-bit view has them
/// (`RegistryDefaults.CurrentVersion.yaml` `folders_64`).
const CURRENT_VERSION_64: &[(&str, Data)] = &[
    ("ProgramFilesDir", Sz("C:\\Program Files")),
    ("ProgramFilesDir (x86)", Sz("C:\\Program Files (x86)")),
    ("ProgramW6432Dir", Sz("C:\\Program Files")),
    ("CommonFilesDir", Sz("C:\\Program Files\\Common Files")),
    ("CommonFilesDir (x86)", Sz("C:\\Program Files (x86)\\Common Files")),
    ("CommonW6432Dir", Sz("C:\\Program Files\\Common Files")),
    ("ProgramFilesPath", ExpandSz("%ProgramFiles%")),
    ("DevicePath", ExpandSz("%SystemRoot%\\inf")),
    ("MediaPathUnexpanded", ExpandSz("%SystemRoot%\\Media")),
];

/// The same key as the 32-bit view has it: a 32-bit program's program folders are the `(x86)`
/// ones (`folders_32`).
const CURRENT_VERSION_32: &[(&str, Data)] = &[
    ("ProgramFilesDir", Sz("C:\\Program Files (x86)")),
    ("ProgramFilesDir (x86)", Sz("C:\\Program Files (x86)")),
    ("ProgramW6432Dir", Sz("C:\\Program Files")),
    ("CommonFilesDir", Sz("C:\\Program Files (x86)\\Common Files")),
    ("CommonFilesDir (x86)", Sz("C:\\Program Files (x86)\\Common Files")),
    ("CommonW6432Dir", Sz("C:\\Program Files\\Common Files")),
    ("ProgramFilesPath", ExpandSz("%ProgramFiles%")),
    ("DevicePath", ExpandSz("%SystemRoot%\\inf")),
    ("MediaPathUnexpanded", ExpandSz("%SystemRoot%\\Media")),
];

/// `CurrentVersion\Installer` in each view (`installer`).
const INSTALLER_64: &[(&str, Data)] = &[
    ("InstallerLocation", Sz("C:\\Windows\\System32\\")),
    ("MsiExecCA32", Sz("C:\\Windows\\syswow64\\msiexec.exe")),
    ("MsiExecCA64", Sz("C:\\Windows\\System32\\msiexec.exe")),
];
const INSTALLER_32: &[(&str, Data)] = &[("InstallerLocation", Sz("C:\\Windows\\SysWOW64\\"))];

/// The keys of the 32-bit view of `HKLM\SOFTWARE` other than `Classes`, below the physical
/// `WOW6432Node` (`RegistryDefaults.Wow6432Node.yaml` `software_64`, `microsoft`;
/// `RegistryDefaults.CurrentVersion.yaml`). `Windows NT\CurrentVersion` is copied there from
/// [`SOFTWARE_KEYS`] (the same values in both views).
const SOFTWARE_32_KEYS: &[DefaultKey] = &[
    k("Clients", &[]),
    k("Policies", &[]),
    k("RegisteredApplications", &[]),
    k("Microsoft\\Ole", &[]),
    k("Microsoft\\COM3", &[]),
    k("Microsoft\\Rpc", &[]),
    k("Microsoft\\Windows\\CurrentVersion", CURRENT_VERSION_32),
    k("Microsoft\\Windows\\CurrentVersion\\Installer", INSTALLER_32),
    k("Microsoft\\Windows\\CurrentVersion\\App Paths", &[]),
    k("Microsoft\\Windows\\CurrentVersion\\Run", &[]),
    k("Microsoft\\Windows\\CurrentVersion\\RunOnce", &[]),
    k("Microsoft\\Windows\\CurrentVersion\\Uninstall", &[]),
    k("Microsoft\\Windows\\CurrentVersion\\Policies", &[]),
    k("Microsoft\\Windows\\CurrentVersion\\Explorer", &[]),
    k("Microsoft\\Windows\\CurrentVersion\\Setup", &[]),
    k("Microsoft\\Windows\\CurrentVersion\\Internet Settings", &[]),
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
    // A vendor key that exists on a machine where Google software has run (Google Update,
    // Chrome): Chrome for Testing finds `Software\Google` already there (call-site trace of
    // chrome_elf's start-up on Windows, ADR 0155).
    k("Software\\Google", &[]),
    k("Software\\Microsoft\\Windows\\CurrentVersion\\Run", &[]),
];

/// What follows the user's SID in the name of the user's classes hive below
/// `HKEY_USERS` (`<SID>_Classes`).
pub const USER_CLASSES_SUFFIX: &str = "_Classes";

/// The user's `Volatile Environment` (Learn: "Recognized environment
/// variables" names each).
pub const VOLATILE_ENVIRONMENT: &[(&str, Data)] = &[
    ("APPDATA", Sz("C:\\Users\\poly\\AppData\\Roaming")),
    ("HOMEDRIVE", Sz("C:")),
    ("HOMEPATH", Sz("\\Users\\poly")),
    ("LOCALAPPDATA", Sz("C:\\Users\\poly\\AppData\\Local")),
    ("LOGONSERVER", Sz("\\\\POLY")),
    ("USERDOMAIN", Sz(POLY_COMPUTER_NAME)),
    // The domain of a roaming profile: the machine's own on a workgroup machine (the sheet:
    // present beside `USERDOMAIN`).
    ("USERDOMAIN_ROAMINGPROFILE", Sz(POLY_COMPUTER_NAME)),
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

/// The `Classes` subkeys whose 32-bit view lives below `Classes\WOW6432Node` (Learn: "Registry
/// Keys Affected by WOW64": the others, `TypeLib` and `AppID` among them, are shared).
const CLASSES_REDIRECTED: [&str; 5] = ["CLSID", "Interface", "DirectShow", "Media Type", "MediaFoundation"];

/// Make each of `keys` below `prefix` (a path below `root`) if it is missing, and give a key
/// that was made its values; a key the hive has already keeps its own.
fn seed(reg: &mut Registry, root: KeyId, prefix: &str, keys: &[DefaultKey], now: u64) -> Result<(), NtStatus> {
    let base = make_path(reg, root, prefix, false, now)?;
    for key in keys {
        let mut cur = base;
        let mut created = false;
        for c in key.path.split('\\').filter(|c| !c.is_empty()) {
            let (k, d) = reg.create_key(cur, &wide(c), CreateOptions::default(), now)?;
            (cur, created) = (k, d == crate::tree::Disposition::Created);
        }
        if created {
            set(reg, cur, key.values, now)?;
        }
    }
    Ok(())
}

/// The keys of both registry views in the software hive `root` (ADR 0186 addenda; the sheets of
/// `spec-win-wow64-defaults`, `docs/windows/registry-defaults.md` section 6), made in a new hive
/// and in one loaded from a file older than them alike; a key the hive has keeps its values:
///
/// - `WOW6432Node` (the 32-bit view of `HKLM\SOFTWARE`) with [`SOFTWARE_32_KEYS`] and a copy of
///   `Microsoft\Windows NT\CurrentVersion` (the same values in both views).
/// - `Classes\WOW6432Node` with the five redirected `Classes` subkeys (`TypeLib` and `AppID`
///   there, and `WOW6432Node\Classes`, are links: [`volatile_keys`]).
/// - The standard COM registrations ([`crate::comdefaults`]): the interfaces in both views, the
///   proxy/stub classes with each view's server paths, `stdole` once (shared).
fn wow64_view_keys(reg: &mut Registry, root: KeyId, now: u64) -> Result<(), NtStatus> {
    use crate::comdefaults::{CLASSES_32, CLASSES_64, INTERFACES, STDOLE};
    seed(reg, root, WOW6432_NODE, SOFTWARE_32_KEYS, now)?;
    let nt = "Microsoft\\Windows NT\\CurrentVersion";
    if let Some(key) = SOFTWARE_KEYS.iter().find(|k| k.path == nt) {
        let node_nt = alloc::format!("{WOW6432_NODE}\\{nt}");
        let made = reg.find(root, &node_nt).is_none();
        let id = make_path(reg, root, &node_nt, false, now)?;
        if made {
            set(reg, id, key.values, now)?;
            // The install date of the 64-bit key, or this boot for a hive without one.
            let date = reg
                .find(root, nt)
                .and_then(|k| reg.value(k, &wide("InstallDate")).ok().map(|v| v.data.clone()))
                .unwrap_or_else(|| install_date(now).to_le_bytes().to_vec());
            reg.set_value(id, &wide("InstallDate"), REG_DWORD, &date, now)?;
        }
    }
    let classes_node = alloc::format!("Classes\\{WOW6432_NODE}");
    for sub in CLASSES_REDIRECTED {
        make_path(reg, root, &alloc::format!("{classes_node}\\{sub}"), false, now)?;
    }
    seed(reg, root, "Classes", INTERFACES, now)?;
    seed(reg, root, &classes_node, INTERFACES, now)?;
    seed(reg, root, "Classes", CLASSES_64, now)?;
    seed(reg, root, &classes_node, CLASSES_32, now)?;
    seed(reg, root, "Classes", STDOLE, now)?;
    Ok(())
}

/// `InstallDate`: seconds since 1970 at `now` (a FILETIME).
fn install_date(now: u64) -> u32 {
    (now.saturating_sub(FILETIME_UNIX_EPOCH) / 10_000_000) as u32
}

/// A volatile symbolic link `name` below `parent` to the NT path `target`, unless `parent` has a
/// key of that name already (a hive from a file where a program made one keeps it).
fn link(reg: &mut Registry, parent: KeyId, name: &str, target: &str, now: u64) -> Result<(), NtStatus> {
    if reg.child(parent, &wide(name)).is_some() {
        return Ok(());
    }
    let opts = CreateOptions { volatile: true, link: true, class: &[] };
    let (key, _) = reg.create_key(parent, &wide(name), opts, now)?;
    let bytes: Vec<u8> = target.encode_utf16().flat_map(u16::to_le_bytes).collect();
    reg.set_value(key, &wide(SYMBOLIC_LINK_VALUE), REG_LINK, &bytes, now)
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
            reg.set_value(id, &wide("InstallDate"), REG_DWORD, &install_date(now).to_le_bytes(), now)?;
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
        if spec.file == "SOFTWARE" {
            let root = reg.hive(id).root;
            wow64_view_keys(reg, root, now)?;
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
    // The names Windows keeps for one key in the physical registry (`RegistryDefaults.Wow6432Node.yaml`
    // `one_key_64`): `SOFTWARE\WOW6432Node\Classes` is `SOFTWARE\Classes\WOW6432Node`, and below
    // that `TypeLib` and `AppID` are the shared `Classes\TypeLib` and `Classes\AppID`. (The 32-bit
    // view reaches the same keys by its own rule: `personalities/nt/src/registry/wow64.rs`.)
    if let Some(software) = reg.find(machine, "SOFTWARE") {
        let node = alloc::format!("\\REGISTRY\\MACHINE\\SOFTWARE\\Classes\\{WOW6432_NODE}");
        if let Some(wow) = reg.find(software, WOW6432_NODE) {
            link(reg, wow, "Classes", &node, now)?;
        }
        if let Some(classes_node) = reg.find(software, &alloc::format!("Classes\\{WOW6432_NODE}")) {
            for shared in ["TypeLib", "AppID"] {
                link(
                    reg,
                    classes_node,
                    shared,
                    &alloc::format!("\\REGISTRY\\MACHINE\\SOFTWARE\\Classes\\{shared}"),
                    now,
                )?;
            }
        }
    }
    let user = reg.user();
    if let Some(me) = reg.find(user, POLY_USER_SID) {
        let env = make_path(reg, me, "Volatile Environment", true, now)?;
        set(reg, env, VOLATILE_ENVIRONMENT, now)?;
        // The 32-bit view of the user's redirected classes (`RegistryDefaults.ClassesRoots.yaml`
        // `hkcu_clsid`: a 32-bit program makes `Software\Classes\CLSID\{id}` there).
        for sub in ["CLSID", "Interface"] {
            make_path(reg, me, &alloc::format!("Software\\Classes\\{WOW6432_NODE}\\{sub}"), false, now)?;
        }
    }
    // `\REGISTRY\USER\<user>_Classes`, the user's classes hive (RtlFormatCurrentUserKeyPath.yaml
    // `usable`: it opens). Poly keeps the user's classes in `<user>\Software\Classes`, so the
    // name is a link there and both reach the same keys.
    let classes = alloc::format!("{POLY_USER_SID}{USER_CLASSES_SUFFIX}");
    if reg.child(user, &wide(&classes)).is_none() {
        let opts = CreateOptions { volatile: true, link: true, class: &[] };
        let (link, _) = reg.create_key(user, &wide(&classes), opts, now)?;
        let target: Vec<u8> = alloc::format!("\\REGISTRY\\USER\\{POLY_USER_SID}\\Software\\Classes")
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        reg.set_value(link, &wide(SYMBOLIC_LINK_VALUE), REG_LINK, &target, now)?;
    }
    Ok(())
}
