//! The Windows file system layout Poly presents on `C:` ([ADR 0073]): the
//! folders, which known folder, environment variable and registry value
//! name each one, making them on the VFS, keeping the registry in step with
//! them, and the environment block a new NT process gets.
//!
//! The table is Poly's own, written from public documentation only:
//! Microsoft Learn's "KNOWNFOLDERID" reference (each folder's default path
//! and its `CSIDL` / legacy shell-folder name), "Recognized environment
//! variables", "WOW64 Implementation Details" (the program-files variables
//! and registry values a 64-bit process sees), "File System Redirector",
//! "User Profiles" (`GetProfilesDirectory`, `GetDefaultUserProfileDirectory`,
//! `GetAllUsersProfileDirectory`) and "Naming Files, Paths, and Namespaces".
//!
//! - [`FOLDERS`]: every folder made on the system drive, machine-wide or
//!   below the user's profile ([`Base`]).
//! - [`create`]: make them on the VFS, idempotently, finding existing
//!   folders without case.
//! - [`sync`]: the registry values that must agree with the layout.
//! - [`env`]: the default environment of an NT process, from the registry.
//!
//! The process parameters that carry the environment into a process are
//! [`crate::params`].
//!
//! [ADR 0073]: ../../../../docs/adr/0073-windows-file-system-layout.md

pub mod create;
pub mod env;
pub mod sync;

#[cfg(test)]
mod tests;

use alloc::string::String;
use alloc::vec::Vec;

/// The system drive (Learn: `%SystemDrive%`, "the drive containing the
/// Windows directory").
pub const SYSTEM_DRIVE: &str = "C:";
/// `%SystemRoot%` / `%windir%`, `FOLDERID_Windows`.
pub const SYSTEM_ROOT: &str = "C:\\Windows";
/// `FOLDERID_System`: where Poly's own system DLLs live, the loader's
/// system directory (ADR 0071's search order, ADR 0073).
pub const SYSTEM32: &str = "C:\\Windows\\System32";
/// `%ProgramFiles%`, `FOLDERID_ProgramFiles`.
pub const PROGRAM_FILES: &str = "C:\\Program Files";
/// `%ProgramFiles(x86)%`, `FOLDERID_ProgramFilesX86`.
pub const PROGRAM_FILES_X86: &str = "C:\\Program Files (x86)";
/// `%CommonProgramFiles%`, `FOLDERID_ProgramFilesCommon`.
pub const COMMON_FILES: &str = "C:\\Program Files\\Common Files";
/// `%CommonProgramFiles(x86)%`, `FOLDERID_ProgramFilesCommonX86`.
pub const COMMON_FILES_X86: &str = "C:\\Program Files (x86)\\Common Files";
/// `%ProgramData%` / `%ALLUSERSPROFILE%`, `FOLDERID_ProgramData`.
pub const PROGRAM_DATA: &str = "C:\\ProgramData";
/// `FOLDERID_UserProfiles` (`GetProfilesDirectory`).
pub const USERS: &str = "C:\\Users";
/// `%PUBLIC%`, `FOLDERID_Public`.
pub const PUBLIC: &str = "C:\\Users\\Public";
/// The profile new users are copied from (`GetDefaultUserProfileDirectory`).
pub const DEFAULT_PROFILE: &str = "C:\\Users\\Default";

/// A user with a profile on the system drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct User {
    /// The account name (`%USERNAME%`), also the profile folder's name.
    pub name: &'static str,
    /// Its security identifier: `HKEY_USERS\<sid>` is its hive.
    pub sid: &'static str,
}

/// Poly's single user (ADR 0066): `poly`, `S-1-5-21-0-0-0-1000`.
pub const POLY_USER: User =
    User { name: poly_registry::defaults::POLY_USER_NAME, sid: poly_registry::defaults::POLY_USER_SID };

impl User {
    /// `%USERPROFILE%`: `C:\Users\<name>`.
    pub fn profile(&self) -> String {
        alloc::format!("{USERS}\\{}", self.name)
    }

    /// `%HOMEPATH%`: the profile without the drive.
    pub fn home_path(&self) -> String {
        alloc::format!("\\Users\\{}", self.name)
    }

    /// `%APPDATA%`, `FOLDERID_RoamingAppData`.
    pub fn app_data(&self) -> String {
        alloc::format!("{}\\AppData\\Roaming", self.profile())
    }

    /// `%LOCALAPPDATA%`, `FOLDERID_LocalAppData`.
    pub fn local_app_data(&self) -> String {
        alloc::format!("{}\\AppData\\Local", self.profile())
    }

    /// The user's `%TEMP%` and `%TMP%` (Learn: "GetTempPath" reads them).
    pub fn temp(&self) -> String {
        alloc::format!("{}\\Temp", self.local_app_data())
    }
}

/// What a folder's path is relative to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    /// The system drive's root (`C:\`): machine-wide.
    Drive,
    /// The user's profile (`%USERPROFILE%`): one per user.
    Profile,
}

/// One folder of the layout.
#[derive(Clone, Copy, Debug)]
pub struct Folder {
    pub base: Base,
    /// The path below [`Self::base`], `\`-separated, in the case it is made
    /// with; empty for the profile itself.
    pub path: &'static str,
    /// The `KNOWNFOLDERID` naming it (Learn: "KNOWNFOLDERID"), if any.
    pub known: Option<&'static str>,
    /// The environment variables whose value is this folder.
    pub env: &'static [&'static str],
    /// Its value name under `Explorer\Shell Folders` and `Explorer\User
    /// Shell Folders` (the legacy name the KNOWNFOLDERID reference lists
    /// beside each `CSIDL`), if it has one.
    pub shell: Option<&'static str>,
    /// Made with `FILE_ATTRIBUTE_HIDDEN` (as Windows shows `ProgramData`,
    /// `AppData` and the default profile; to be confirmed by the spec
    /// agent).
    pub hidden: bool,
}

const fn drive(path: &'static str, known: Option<&'static str>) -> Folder {
    Folder { base: Base::Drive, path, known, env: &[], shell: None, hidden: false }
}

const fn profile(path: &'static str, known: Option<&'static str>) -> Folder {
    Folder { base: Base::Profile, path, known, env: &[], shell: None, hidden: false }
}

const fn with_env(mut f: Folder, env: &'static [&'static str]) -> Folder {
    f.env = env;
    f
}

const fn with_shell(mut f: Folder, shell: &'static str) -> Folder {
    f.shell = Some(shell);
    f
}

const fn hidden(mut f: Folder) -> Folder {
    f.hidden = true;
    f
}

/// Every folder of the layout, parents before children. Paths and names
/// are Learn's (see the module documentation); which ones Poly makes at
/// all is ADR 0073's choice.
pub const FOLDERS: &[Folder] = &[
    // The Windows directory.
    with_env(drive("Windows", Some("FOLDERID_Windows")), &["SystemRoot", "windir"]),
    drive("Windows\\System32", Some("FOLDERID_System")),
    drive("Windows\\System32\\drivers", None),
    drive("Windows\\System32\\drivers\\etc", None),
    // Learn ("File System Redirector"): 32-bit programs' system directory
    // on 64-bit Windows. Poly runs 64-bit programs only, so nothing is
    // redirected; it exists for programs that look for it.
    drive("Windows\\SysWOW64", Some("FOLDERID_SystemX86")),
    drive("Windows\\Fonts", Some("FOLDERID_Fonts")),
    // The system TEMP of `Session Manager\Environment` (ADR 0066).
    drive("Windows\\Temp", None),
    // Side-by-side assemblies (ADR 0055: the VC++ 2005/2008 runtimes).
    drive("Windows\\WinSxS", None),
    // Programs.
    with_env(drive("Program Files", Some("FOLDERID_ProgramFiles")), &["ProgramFiles", "ProgramW6432"]),
    with_env(
        drive("Program Files\\Common Files", Some("FOLDERID_ProgramFilesCommon")),
        &["CommonProgramFiles", "CommonProgramW6432"],
    ),
    with_env(drive("Program Files (x86)", Some("FOLDERID_ProgramFilesX86")), &["ProgramFiles(x86)"]),
    with_env(
        drive("Program Files (x86)\\Common Files", Some("FOLDERID_ProgramFilesCommonX86")),
        &["CommonProgramFiles(x86)"],
    ),
    // Machine-wide program data.
    hidden(with_shell(
        with_env(drive("ProgramData", Some("FOLDERID_ProgramData")), &["ProgramData", "ALLUSERSPROFILE"]),
        "Common AppData",
    )),
    with_shell(
        drive("ProgramData\\Microsoft\\Windows\\Start Menu", Some("FOLDERID_CommonStartMenu")),
        "Common Start Menu",
    ),
    with_shell(
        drive("ProgramData\\Microsoft\\Windows\\Start Menu\\Programs", Some("FOLDERID_CommonPrograms")),
        "Common Programs",
    ),
    with_shell(
        drive("ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\StartUp", Some("FOLDERID_CommonStartup")),
        "Common Startup",
    ),
    with_shell(
        drive("ProgramData\\Microsoft\\Windows\\Templates", Some("FOLDERID_CommonTemplates")),
        "Common Templates",
    ),
    // Profiles.
    drive("Users", Some("FOLDERID_UserProfiles")),
    with_env(drive("Users\\Public", Some("FOLDERID_Public")), &["PUBLIC"]),
    with_shell(drive("Users\\Public\\Desktop", Some("FOLDERID_PublicDesktop")), "Common Desktop"),
    with_shell(drive("Users\\Public\\Documents", Some("FOLDERID_PublicDocuments")), "Common Documents"),
    drive("Users\\Public\\Downloads", Some("FOLDERID_PublicDownloads")),
    with_shell(drive("Users\\Public\\Music", Some("FOLDERID_PublicMusic")), "CommonMusic"),
    with_shell(drive("Users\\Public\\Pictures", Some("FOLDERID_PublicPictures")), "CommonPictures"),
    with_shell(drive("Users\\Public\\Videos", Some("FOLDERID_PublicVideos")), "CommonVideo"),
    hidden(drive("Users\\Default", None)),
    // The user's profile.
    with_env(profile("", Some("FOLDERID_Profile")), &["USERPROFILE"]),
    with_shell(profile("Desktop", Some("FOLDERID_Desktop")), "Desktop"),
    with_shell(profile("Documents", Some("FOLDERID_Documents")), "Personal"),
    profile("Downloads", Some("FOLDERID_Downloads")),
    with_shell(profile("Music", Some("FOLDERID_Music")), "My Music"),
    with_shell(profile("Pictures", Some("FOLDERID_Pictures")), "My Pictures"),
    with_shell(profile("Videos", Some("FOLDERID_Videos")), "My Video"),
    with_shell(profile("Favorites", Some("FOLDERID_Favorites")), "Favorites"),
    hidden(profile("AppData", None)),
    with_shell(with_env(profile("AppData\\Roaming", Some("FOLDERID_RoamingAppData")), &["APPDATA"]), "AppData"),
    with_shell(with_env(profile("AppData\\Local", Some("FOLDERID_LocalAppData")), &["LOCALAPPDATA"]), "Local AppData"),
    with_env(profile("AppData\\Local\\Temp", None), &["TEMP", "TMP"]),
    profile("AppData\\LocalLow", Some("FOLDERID_LocalAppDataLow")),
    with_shell(profile("AppData\\Roaming\\Microsoft\\Windows\\Start Menu", Some("FOLDERID_StartMenu")), "Start Menu"),
    with_shell(
        profile("AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs", Some("FOLDERID_Programs")),
        "Programs",
    ),
    with_shell(
        profile("AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\StartUp", Some("FOLDERID_Startup")),
        "Startup",
    ),
    with_shell(profile("AppData\\Roaming\\Microsoft\\Windows\\SendTo", Some("FOLDERID_SendTo")), "SendTo"),
    with_shell(profile("AppData\\Roaming\\Microsoft\\Windows\\Templates", Some("FOLDERID_Templates")), "Templates"),
    with_shell(profile("AppData\\Roaming\\Microsoft\\Windows\\Recent", Some("FOLDERID_Recent")), "Recent"),
];

impl Folder {
    /// The path below the drive root (no drive, no leading `\`).
    pub fn below_drive(&self, user: &User) -> String {
        match self.base {
            Base::Drive => self.path.into(),
            Base::Profile if self.path.is_empty() => alloc::format!("Users\\{}", user.name),
            Base::Profile => alloc::format!("Users\\{}\\{}", user.name, self.path),
        }
    }

    /// The full DOS path, `C:\...`.
    pub fn dos_path(&self, user: &User) -> String {
        alloc::format!("{SYSTEM_DRIVE}\\{}", self.below_drive(user))
    }

    /// The path as `User Shell Folders` stores it (`REG_EXPAND_SZ`): below
    /// `%USERPROFILE%` for the user's folders, below `%ProgramData%` or
    /// `%PUBLIC%` for the machine's.
    pub fn unexpanded(&self, user: &User) -> String {
        match self.base {
            Base::Profile if self.path.is_empty() => "%USERPROFILE%".into(),
            Base::Profile => alloc::format!("%USERPROFILE%\\{}", self.path),
            Base::Drive => {
                let full = self.dos_path(user);
                for (prefix, var) in [(PROGRAM_DATA, "%ProgramData%"), (PUBLIC, "%PUBLIC%")] {
                    if let Some(rest) = full.strip_prefix(prefix) {
                        return alloc::format!("{var}{rest}");
                    }
                }
                full
            }
        }
    }
}

/// The folder an environment variable names, if the layout has one
/// (without case, as variable names are).
pub fn folder_of_variable(name: &str) -> Option<&'static Folder> {
    FOLDERS.iter().find(|f| f.env.iter().any(|e| e.eq_ignore_ascii_case(name)))
}

/// The folder a `KNOWNFOLDERID` names, if the layout has one.
pub fn known_folder(id: &str) -> Option<&'static Folder> {
    FOLDERS.iter().find(|f| f.known == Some(id))
}

/// An NT-side path for a VFS path (an executable a Linux program started):
/// below `c_root` (the VFS directory behind `C:`) it is `C:\...`, anything
/// else is on `Z:` (ADR 0067: `Z:` is the whole VFS). `/` becomes `\`.
/// Bytes that are not UTF-8 are shown as U+FFFD.
pub fn dos_path_of_vfs(path: &[u8], c_roots: &[&[u8]]) -> String {
    let path = String::from_utf8_lossy(path);
    let path = path.as_ref();
    for root in c_roots {
        let root = String::from_utf8_lossy(root);
        let root = root.trim_end_matches('/');
        if root.is_empty() {
            continue;
        }
        if let Some(rest) = path.strip_prefix(root)
            && (rest.is_empty() || rest.starts_with('/'))
        {
            return alloc::format!("{SYSTEM_DRIVE}\\{}", rest.trim_start_matches('/').replace('/', "\\"));
        }
    }
    let rest = path.trim_start_matches('/');
    alloc::format!("Z:\\{}", rest.replace('/', "\\"))
}

/// The folders of the layout as full DOS paths, in [`FOLDERS`] order.
pub fn dos_paths(user: &User) -> Vec<String> {
    FOLDERS.iter().map(|f| f.dos_path(user)).collect()
}
