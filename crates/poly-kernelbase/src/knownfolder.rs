//! The known folders of `SHGetKnownFolderPath` (ADR 0155, step 9): which `KNOWNFOLDERID` names
//! which folder. Chrome's browser asks for one when it starts, and treats a failure as fatal.
//!
//! Sources: Learn, "KNOWNFOLDERID" (the GUID and the default path of each folder; the table
//! below is that page's, with the folders that have a path), "SHGetKnownFolderPath function
//! (shlobj_core.h)" (the path has no trailing backslash; `E_FAIL` for a folder without a path,
//! `E_INVALIDARG` for an id the system does not have) and "KNOWN_FOLDER_FLAG". The default
//! paths start with an environment variable, which is how Poly's session lays the profile out
//! (ADR 0073); the 64-bit program-files and system folders are spelled the way Poly's layout
//! has them. Doc-derived, not observed on Windows.

use alloc::vec::Vec;

/// `S_OK` (`winerror.h`).
pub const S_OK: u32 = 0;
/// `E_FAIL` (`winerror.h`).
pub const E_FAIL: u32 = 0x8000_4005;
/// `E_INVALIDARG` (`winerror.h`).
pub const E_INVALIDARG: u32 = 0x8007_0057;
/// `E_POINTER` (`winerror.h`).
pub const E_POINTER: u32 = 0x8000_4003;
/// `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)`: no such known folder, or its directory is not
/// there (`specs/windows/shell32.dll/SHGetKnownFolderPath.yaml`).
pub const E_FILE_NOT_FOUND: u32 = 0x8007_0002;
/// `E_ACCESSDENIED` (`winerror.h`): a token without `TOKEN_QUERY`.
pub const E_ACCESSDENIED: u32 = 0x8007_0005;
/// `E_HANDLE` (`winerror.h`): `hToken` is no token handle.
pub const E_HANDLE: u32 = 0x8007_0006;
/// `E_OUTOFMEMORY` (`winerror.h`).
pub const E_OUTOFMEMORY: u32 = 0x8007_000E;
/// `HRESULT_FROM_WIN32(ERROR_PATH_NOT_FOUND)`: the folder's path cannot be made (a variable
/// of its default path is not set).
pub const E_PATH_NOT_FOUND: u32 = 0x8007_0003;

/// `KF_FLAG_CREATE` (`KNOWN_FOLDER_FLAG`, `shlobj_core.h`): "Forces the creation of the
/// specified folder if that folder does not already exist."
pub const KF_FLAG_CREATE: u32 = 0x0000_8000;
/// `KF_FLAG_DONT_VERIFY`: "Do not verify the folder's existence".
pub const KF_FLAG_DONT_VERIFY: u32 = 0x0000_4000;

/// A known folder: its name without `FOLDERID_`, its GUID as the 16 bytes a `KNOWNFOLDERID`
/// has in memory, and its default path (starting with `%VARIABLE%`; empty for a folder that
/// has no path: the virtual folders).
pub type KnownFolder = (&'static str, [u8; 16], &'static str);

/// The folder a `KNOWNFOLDERID` names.
pub fn lookup(id: &[u8; 16]) -> Option<&'static KnownFolder> {
    KNOWN_FOLDERS.iter().find(|(_, guid, _)| guid == id)
}

/// The path of a folder, from its default path and the environment (`variable(name)` is the
/// value of a variable, without a NUL): `E_FAIL` for a folder without a path,
/// `E_PATH_NOT_FOUND` when its variable is not set. No trailing backslash.
pub fn path(folder: &KnownFolder, variable: impl Fn(&str) -> Option<Vec<u16>>) -> Result<Vec<u16>, u32> {
    let template = folder.2;
    let rest = template.strip_prefix('%').ok_or(E_FAIL)?;
    let (name, suffix) = rest.split_once('%').ok_or(E_FAIL)?;
    let mut out = variable(name).filter(|v| !v.is_empty()).ok_or(E_PATH_NOT_FOUND)?;
    while out.len() > 3 && out.last() == Some(&u16::from(b'\\')) {
        out.pop();
    }
    out.extend(suffix.encode_utf16());
    Ok(out)
}

/// `KF_FLAG_NOT_PARENT_RELATIVE`, `KF_FLAG_DEFAULT_PATH`, `KF_FLAG_ALIAS_ONLY`
/// (`KNOWN_FOLDER_FLAG`, `shlobj_core.h`).
pub const KF_FLAG_NOT_PARENT_RELATIVE: u32 = 0x200;
pub const KF_FLAG_DEFAULT_PATH: u32 = 0x400;
pub const KF_FLAG_ALIAS_ONLY: u32 = 0x8000_0000;

/// Whether `SHGetKnownFolderPath` takes these flags
/// (`specs/windows/shell32.dll/SHGetKnownFolderPath.yaml`, rule `flags_refused`):
/// `KF_FLAG_NOT_PARENT_RELATIVE` without `KF_FLAG_DEFAULT_PATH`, and `KF_FLAG_ALIAS_ONLY`, are
/// `E_INVALIDARG`; the other flags change nothing on a folder that exists.
pub fn flags_accepted(flags: u32) -> bool {
    let lonely = flags & KF_FLAG_NOT_PARENT_RELATIVE != 0 && flags & KF_FLAG_DEFAULT_PATH == 0;
    !lonely && flags & KF_FLAG_ALIAS_ONLY == 0
}

/// `SHGetKnownFolderPath`'s answer for an id: the path, or the `HRESULT`. An id that is no known
/// folder (also the all-zero GUID) is `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)`, as the sheet
/// observed.
pub fn known_folder_path(id: &[u8; 16], variable: impl Fn(&str) -> Option<Vec<u16>>) -> Result<Vec<u16>, u32> {
    path(lookup(id).ok_or(E_FILE_NOT_FOUND)?, variable)
}

/// The path of a folder for the default user (`hToken` -1): a folder below the user's profile
/// moves to the same place below the Default profile (a sibling of it in the directory the
/// profiles are in); every other folder is unchanged (sheet rule `token`: Windows is unchanged).
pub fn for_default_user(path: &[u16], profile: &[u16]) -> Vec<u16> {
    let backslash = u16::from(b'\\');
    let mut profile = profile;
    while let [rest @ .., last] = profile
        && *last == backslash
        && rest.len() > 2
    {
        profile = rest;
    }
    let lower = |c: u16| if (u16::from(b'A')..=u16::from(b'Z')).contains(&c) { c + 32 } else { c };
    let below = !profile.is_empty()
        && path.len() >= profile.len()
        && path[..profile.len()].iter().zip(profile).all(|(a, b)| lower(*a) == lower(*b))
        && (path.len() == profile.len() || path[profile.len()] == backslash);
    if !below {
        return path.to_vec();
    }
    let Some(slash) = profile.iter().rposition(|&c| c == backslash) else { return path.to_vec() };
    let mut out = profile[..slash + 1].to_vec();
    out.extend("Default".encode_utf16());
    out.extend_from_slice(&path[profile.len()..]);
    out
}

/// Every known folder of Learn's "KNOWNFOLDERID" page.
pub const KNOWN_FOLDERS: &[KnownFolder] = &[
    // FOLDERID_AccountPictures {008CA0B1-55B4-4C56-B8A8-4DE4B299D3BE}
    (
        "AccountPictures",
        [0xB1, 0xA0, 0x8C, 0x00, 0xB4, 0x55, 0x56, 0x4C, 0xB8, 0xA8, 0x4D, 0xE4, 0xB2, 0x99, 0xD3, 0xBE],
        "%APPDATA%\\Microsoft\\Windows\\AccountPictures",
    ),
    // FOLDERID_AddNewPrograms {DE61D971-5EBC-4F02-A3A9-6C82895E5C04}
    (
        "AddNewPrograms",
        [0x71, 0xD9, 0x61, 0xDE, 0xBC, 0x5E, 0x02, 0x4F, 0xA3, 0xA9, 0x6C, 0x82, 0x89, 0x5E, 0x5C, 0x04],
        "",
    ),
    // FOLDERID_AdminTools {724EF170-A42D-4FEF-9F26-B60E846FBA4F}
    (
        "AdminTools",
        [0x70, 0xF1, 0x4E, 0x72, 0x2D, 0xA4, 0xEF, 0x4F, 0x9F, 0x26, 0xB6, 0x0E, 0x84, 0x6F, 0xBA, 0x4F],
        "%APPDATA%\\Microsoft\\Windows\\Start Menu\\Programs\\Administrative Tools",
    ),
    // FOLDERID_AppDataDesktop {B2C5E279-7ADD-439F-B28C-C41FE1BBF672}
    (
        "AppDataDesktop",
        [0x79, 0xE2, 0xC5, 0xB2, 0xDD, 0x7A, 0x9F, 0x43, 0xB2, 0x8C, 0xC4, 0x1F, 0xE1, 0xBB, 0xF6, 0x72],
        "%LOCALAPPDATA%\\Desktop",
    ),
    // FOLDERID_AppDataDocuments {7BE16610-1F7F-44AC-BFF0-83E15F2FFCA1}
    (
        "AppDataDocuments",
        [0x10, 0x66, 0xE1, 0x7B, 0x7F, 0x1F, 0xAC, 0x44, 0xBF, 0xF0, 0x83, 0xE1, 0x5F, 0x2F, 0xFC, 0xA1],
        "%LOCALAPPDATA%\\Documents",
    ),
    // FOLDERID_AppDataFavorites {7CFBEFBC-DE1F-45AA-B843-A542AC536CC9}
    (
        "AppDataFavorites",
        [0xBC, 0xEF, 0xFB, 0x7C, 0x1F, 0xDE, 0xAA, 0x45, 0xB8, 0x43, 0xA5, 0x42, 0xAC, 0x53, 0x6C, 0xC9],
        "%LOCALAPPDATA%\\Favorites",
    ),
    // FOLDERID_AppDataProgramData {559D40A3-A036-40FA-AF61-84CB430A4D34}
    (
        "AppDataProgramData",
        [0xA3, 0x40, 0x9D, 0x55, 0x36, 0xA0, 0xFA, 0x40, 0xAF, 0x61, 0x84, 0xCB, 0x43, 0x0A, 0x4D, 0x34],
        "%LOCALAPPDATA%\\ProgramData",
    ),
    // FOLDERID_AppUpdates {A305CE99-F527-492B-8B1A-7E76FA98D6E4}
    (
        "AppUpdates",
        [0x99, 0xCE, 0x05, 0xA3, 0x27, 0xF5, 0x2B, 0x49, 0x8B, 0x1A, 0x7E, 0x76, 0xFA, 0x98, 0xD6, 0xE4],
        "",
    ),
    // FOLDERID_ApplicationShortcuts {A3918781-E5F2-4890-B3D9-A7E54332328C}
    (
        "ApplicationShortcuts",
        [0x81, 0x87, 0x91, 0xA3, 0xF2, 0xE5, 0x90, 0x48, 0xB3, 0xD9, 0xA7, 0xE5, 0x43, 0x32, 0x32, 0x8C],
        "%LOCALAPPDATA%\\Microsoft\\Windows\\Application Shortcuts",
    ),
    // FOLDERID_AppsFolder {1E87508D-89C2-42F0-8A7E-645A0F50CA58}
    (
        "AppsFolder",
        [0x8D, 0x50, 0x87, 0x1E, 0xC2, 0x89, 0xF0, 0x42, 0x8A, 0x7E, 0x64, 0x5A, 0x0F, 0x50, 0xCA, 0x58],
        "",
    ),
    // FOLDERID_CDBurning {9E52AB10-F80D-49DF-ACB8-4330F5687855}
    (
        "CDBurning",
        [0x10, 0xAB, 0x52, 0x9E, 0x0D, 0xF8, 0xDF, 0x49, 0xAC, 0xB8, 0x43, 0x30, 0xF5, 0x68, 0x78, 0x55],
        "%LOCALAPPDATA%\\Microsoft\\Windows\\Burn\\Burn",
    ),
    // FOLDERID_CameraRoll {AB5FB87B-7CE2-4F83-915D-550846C9537B}
    (
        "CameraRoll",
        [0x7B, 0xB8, 0x5F, 0xAB, 0xE2, 0x7C, 0x83, 0x4F, 0x91, 0x5D, 0x55, 0x08, 0x46, 0xC9, 0x53, 0x7B],
        "%USERPROFILE%\\Pictures\\Camera Roll",
    ),
    // FOLDERID_ChangeRemovePrograms {DF7266AC-9274-4867-8D55-3BD661DE872D}
    (
        "ChangeRemovePrograms",
        [0xAC, 0x66, 0x72, 0xDF, 0x74, 0x92, 0x67, 0x48, 0x8D, 0x55, 0x3B, 0xD6, 0x61, 0xDE, 0x87, 0x2D],
        "",
    ),
    // FOLDERID_CommonAdminTools {D0384E7D-BAC3-4797-8F14-CBA229B392B5}
    (
        "CommonAdminTools",
        [0x7D, 0x4E, 0x38, 0xD0, 0xC3, 0xBA, 0x97, 0x47, 0x8F, 0x14, 0xCB, 0xA2, 0x29, 0xB3, 0x92, 0xB5],
        "%ALLUSERSPROFILE%\\Microsoft\\Windows\\Start Menu\\Programs\\Administrative Tools",
    ),
    // FOLDERID_CommonOEMLinks {C1BAE2D0-10DF-4334-BEDD-7AA20B227A9D}
    (
        "CommonOEMLinks",
        [0xD0, 0xE2, 0xBA, 0xC1, 0xDF, 0x10, 0x34, 0x43, 0xBE, 0xDD, 0x7A, 0xA2, 0x0B, 0x22, 0x7A, 0x9D],
        "%ALLUSERSPROFILE%\\OEM Links",
    ),
    // FOLDERID_CommonPrograms {0139D44E-6AFE-49F2-8690-3DAFCAE6FFB8}
    (
        "CommonPrograms",
        [0x4E, 0xD4, 0x39, 0x01, 0xFE, 0x6A, 0xF2, 0x49, 0x86, 0x90, 0x3D, 0xAF, 0xCA, 0xE6, 0xFF, 0xB8],
        "%ALLUSERSPROFILE%\\Microsoft\\Windows\\Start Menu\\Programs",
    ),
    // FOLDERID_CommonStartMenu {A4115719-D62E-491D-AA7C-E74B8BE3B067}
    (
        "CommonStartMenu",
        [0x19, 0x57, 0x11, 0xA4, 0x2E, 0xD6, 0x1D, 0x49, 0xAA, 0x7C, 0xE7, 0x4B, 0x8B, 0xE3, 0xB0, 0x67],
        "%ALLUSERSPROFILE%\\Microsoft\\Windows\\Start Menu",
    ),
    // FOLDERID_CommonStartup {82A5EA35-D9CD-47C5-9629-E15D2F714E6E}
    (
        "CommonStartup",
        [0x35, 0xEA, 0xA5, 0x82, 0xCD, 0xD9, 0xC5, 0x47, 0x96, 0x29, 0xE1, 0x5D, 0x2F, 0x71, 0x4E, 0x6E],
        "%ALLUSERSPROFILE%\\Microsoft\\Windows\\Start Menu\\Programs\\StartUp",
    ),
    // FOLDERID_CommonTemplates {B94237E7-57AC-4347-9151-B08C6C32D1F7}
    (
        "CommonTemplates",
        [0xE7, 0x37, 0x42, 0xB9, 0xAC, 0x57, 0x47, 0x43, 0x91, 0x51, 0xB0, 0x8C, 0x6C, 0x32, 0xD1, 0xF7],
        "%ALLUSERSPROFILE%\\Microsoft\\Windows\\Templates",
    ),
    // FOLDERID_ComputerFolder {0AC0837C-BBF8-452A-850D-79D08E667CA7}
    (
        "ComputerFolder",
        [0x7C, 0x83, 0xC0, 0x0A, 0xF8, 0xBB, 0x2A, 0x45, 0x85, 0x0D, 0x79, 0xD0, 0x8E, 0x66, 0x7C, 0xA7],
        "",
    ),
    // FOLDERID_ConflictFolder {4BFEFB45-347D-4006-A5BE-AC0CB0567192}
    (
        "ConflictFolder",
        [0x45, 0xFB, 0xFE, 0x4B, 0x7D, 0x34, 0x06, 0x40, 0xA5, 0xBE, 0xAC, 0x0C, 0xB0, 0x56, 0x71, 0x92],
        "",
    ),
    // FOLDERID_ConnectionsFolder {6F0CD92B-2E97-45D1-88FF-B0D186B8DEDD}
    (
        "ConnectionsFolder",
        [0x2B, 0xD9, 0x0C, 0x6F, 0x97, 0x2E, 0xD1, 0x45, 0x88, 0xFF, 0xB0, 0xD1, 0x86, 0xB8, 0xDE, 0xDD],
        "",
    ),
    // FOLDERID_Contacts {56784854-C6CB-462B-8169-88E350ACB882}
    (
        "Contacts",
        [0x54, 0x48, 0x78, 0x56, 0xCB, 0xC6, 0x2B, 0x46, 0x81, 0x69, 0x88, 0xE3, 0x50, 0xAC, 0xB8, 0x82],
        "%USERPROFILE%\\Contacts",
    ),
    // FOLDERID_ControlPanelFolder {82A74AEB-AEB4-465C-A014-D097EE346D63}
    (
        "ControlPanelFolder",
        [0xEB, 0x4A, 0xA7, 0x82, 0xB4, 0xAE, 0x5C, 0x46, 0xA0, 0x14, 0xD0, 0x97, 0xEE, 0x34, 0x6D, 0x63],
        "",
    ),
    // FOLDERID_Cookies {2B0F765D-C0E9-4171-908E-08A611B84FF6}
    (
        "Cookies",
        [0x5D, 0x76, 0x0F, 0x2B, 0xE9, 0xC0, 0x71, 0x41, 0x90, 0x8E, 0x08, 0xA6, 0x11, 0xB8, 0x4F, 0xF6],
        "%APPDATA%\\Microsoft\\Windows\\Cookies",
    ),
    // FOLDERID_Desktop {B4BFCC3A-DB2C-424C-B029-7FE99A87C641}
    (
        "Desktop",
        [0x3A, 0xCC, 0xBF, 0xB4, 0x2C, 0xDB, 0x4C, 0x42, 0xB0, 0x29, 0x7F, 0xE9, 0x9A, 0x87, 0xC6, 0x41],
        "%USERPROFILE%\\Desktop",
    ),
    // FOLDERID_DeviceMetadataStore {5CE4A5E9-E4EB-479D-B89F-130C02886155}
    (
        "DeviceMetadataStore",
        [0xE9, 0xA5, 0xE4, 0x5C, 0xEB, 0xE4, 0x9D, 0x47, 0xB8, 0x9F, 0x13, 0x0C, 0x02, 0x88, 0x61, 0x55],
        "%ALLUSERSPROFILE%\\Microsoft\\Windows\\DeviceMetadataStore",
    ),
    // FOLDERID_Documents {FDD39AD0-238F-46AF-ADB4-6C85480369C7}
    (
        "Documents",
        [0xD0, 0x9A, 0xD3, 0xFD, 0x8F, 0x23, 0xAF, 0x46, 0xAD, 0xB4, 0x6C, 0x85, 0x48, 0x03, 0x69, 0xC7],
        "%USERPROFILE%\\Documents",
    ),
    // FOLDERID_DocumentsLibrary {7B0DB17D-9CD2-4A93-9733-46CC89022E7C}
    (
        "DocumentsLibrary",
        [0x7D, 0xB1, 0x0D, 0x7B, 0xD2, 0x9C, 0x93, 0x4A, 0x97, 0x33, 0x46, 0xCC, 0x89, 0x02, 0x2E, 0x7C],
        "%APPDATA%\\Microsoft\\Windows\\Libraries\\Documents.library-ms",
    ),
    // FOLDERID_Downloads {374DE290-123F-4565-9164-39C4925E467B}
    (
        "Downloads",
        [0x90, 0xE2, 0x4D, 0x37, 0x3F, 0x12, 0x65, 0x45, 0x91, 0x64, 0x39, 0xC4, 0x92, 0x5E, 0x46, 0x7B],
        "%USERPROFILE%\\Downloads",
    ),
    // FOLDERID_Favorites {1777F761-68AD-4D8A-87BD-30B759FA33DD}
    (
        "Favorites",
        [0x61, 0xF7, 0x77, 0x17, 0xAD, 0x68, 0x8A, 0x4D, 0x87, 0xBD, 0x30, 0xB7, 0x59, 0xFA, 0x33, 0xDD],
        "%USERPROFILE%\\Favorites",
    ),
    // FOLDERID_Fonts {FD228CB7-AE11-4AE3-864C-16F3910AB8FE}
    (
        "Fonts",
        [0xB7, 0x8C, 0x22, 0xFD, 0x11, 0xAE, 0xE3, 0x4A, 0x86, 0x4C, 0x16, 0xF3, 0x91, 0x0A, 0xB8, 0xFE],
        "%windir%\\Fonts",
    ),
    // FOLDERID_GameTasks {054FAE61-4DD8-4787-80B6-090220C4B700}
    (
        "GameTasks",
        [0x61, 0xAE, 0x4F, 0x05, 0xD8, 0x4D, 0x87, 0x47, 0x80, 0xB6, 0x09, 0x02, 0x20, 0xC4, 0xB7, 0x00],
        "%LOCALAPPDATA%\\Microsoft\\Windows\\GameExplorer",
    ),
    // FOLDERID_Games {CAC52C1A-B53D-4EDC-92D7-6B2E8AC19434}
    ("Games", [0x1A, 0x2C, 0xC5, 0xCA, 0x3D, 0xB5, 0xDC, 0x4E, 0x92, 0xD7, 0x6B, 0x2E, 0x8A, 0xC1, 0x94, 0x34], ""),
    // FOLDERID_History {D9DC8A3B-B784-432E-A781-5A1130A75963}
    (
        "History",
        [0x3B, 0x8A, 0xDC, 0xD9, 0x84, 0xB7, 0x2E, 0x43, 0xA7, 0x81, 0x5A, 0x11, 0x30, 0xA7, 0x59, 0x63],
        "%LOCALAPPDATA%\\Microsoft\\Windows\\History",
    ),
    // FOLDERID_HomeGroup {52528A6B-B9E3-4ADD-B60D-588C2DBA842D}
    ("HomeGroup", [0x6B, 0x8A, 0x52, 0x52, 0xE3, 0xB9, 0xDD, 0x4A, 0xB6, 0x0D, 0x58, 0x8C, 0x2D, 0xBA, 0x84, 0x2D], ""),
    // FOLDERID_HomeGroupCurrentUser {9B74B6A3-0DFD-4F11-9E78-5F7800F2E772}
    (
        "HomeGroupCurrentUser",
        [0xA3, 0xB6, 0x74, 0x9B, 0xFD, 0x0D, 0x11, 0x4F, 0x9E, 0x78, 0x5F, 0x78, 0x00, 0xF2, 0xE7, 0x72],
        "",
    ),
    // FOLDERID_ImplicitAppShortcuts {BCB5256F-79F6-4CEE-B725-DC34E402FD46}
    (
        "ImplicitAppShortcuts",
        [0x6F, 0x25, 0xB5, 0xBC, 0xF6, 0x79, 0xEE, 0x4C, 0xB7, 0x25, 0xDC, 0x34, 0xE4, 0x02, 0xFD, 0x46],
        "%APPDATA%\\Microsoft\\Internet Explorer\\Quick Launch\\User Pinned\\ImplicitAppShortcuts",
    ),
    // FOLDERID_InternetCache {352481E8-33BE-4251-BA85-6007CAEDCF9D}
    (
        "InternetCache",
        [0xE8, 0x81, 0x24, 0x35, 0xBE, 0x33, 0x51, 0x42, 0xBA, 0x85, 0x60, 0x07, 0xCA, 0xED, 0xCF, 0x9D],
        "%LOCALAPPDATA%\\Microsoft\\Windows\\Temporary Internet Files",
    ),
    // FOLDERID_InternetFolder {4D9F7874-4E0C-4904-967B-40B0D20C3E4B}
    (
        "InternetFolder",
        [0x74, 0x78, 0x9F, 0x4D, 0x0C, 0x4E, 0x04, 0x49, 0x96, 0x7B, 0x40, 0xB0, 0xD2, 0x0C, 0x3E, 0x4B],
        "",
    ),
    // FOLDERID_Libraries {1B3EA5DC-B587-4786-B4EF-BD1DC332AEAE}
    (
        "Libraries",
        [0xDC, 0xA5, 0x3E, 0x1B, 0x87, 0xB5, 0x86, 0x47, 0xB4, 0xEF, 0xBD, 0x1D, 0xC3, 0x32, 0xAE, 0xAE],
        "%APPDATA%\\Microsoft\\Windows\\Libraries",
    ),
    // FOLDERID_Links {BFB9D5E0-C6A9-404C-B2B2-AE6DB6AF4968}
    (
        "Links",
        [0xE0, 0xD5, 0xB9, 0xBF, 0xA9, 0xC6, 0x4C, 0x40, 0xB2, 0xB2, 0xAE, 0x6D, 0xB6, 0xAF, 0x49, 0x68],
        "%USERPROFILE%\\Links",
    ),
    // FOLDERID_LocalAppData {F1B32785-6FBA-4FCF-9D55-7B8E7F157091}
    (
        "LocalAppData",
        [0x85, 0x27, 0xB3, 0xF1, 0xBA, 0x6F, 0xCF, 0x4F, 0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91],
        "%LOCALAPPDATA%",
    ),
    // FOLDERID_LocalAppDataLow {A520A1A4-1780-4FF6-BD18-167343C5AF16}
    (
        "LocalAppDataLow",
        [0xA4, 0xA1, 0x20, 0xA5, 0x80, 0x17, 0xF6, 0x4F, 0xBD, 0x18, 0x16, 0x73, 0x43, 0xC5, 0xAF, 0x16],
        "%USERPROFILE%\\AppData\\LocalLow",
    ),
    // FOLDERID_LocalizedResourcesDir {2A00375E-224C-49DE-B8D1-440DF7EF3DDC}
    (
        "LocalizedResourcesDir",
        [0x5E, 0x37, 0x00, 0x2A, 0x4C, 0x22, 0xDE, 0x49, 0xB8, 0xD1, 0x44, 0x0D, 0xF7, 0xEF, 0x3D, 0xDC],
        "",
    ),
    // FOLDERID_Music {4BD8D571-6D19-48D3-BE97-422220080E43}
    (
        "Music",
        [0x71, 0xD5, 0xD8, 0x4B, 0x19, 0x6D, 0xD3, 0x48, 0xBE, 0x97, 0x42, 0x22, 0x20, 0x08, 0x0E, 0x43],
        "%USERPROFILE%\\Music",
    ),
    // FOLDERID_MusicLibrary {2112AB0A-C86A-4FFE-A368-0DE96E47012E}
    (
        "MusicLibrary",
        [0x0A, 0xAB, 0x12, 0x21, 0x6A, 0xC8, 0xFE, 0x4F, 0xA3, 0x68, 0x0D, 0xE9, 0x6E, 0x47, 0x01, 0x2E],
        "%APPDATA%\\Microsoft\\Windows\\Libraries\\Music.library-ms",
    ),
    // FOLDERID_NetHood {C5ABBF53-E17F-4121-8900-86626FC2C973}
    (
        "NetHood",
        [0x53, 0xBF, 0xAB, 0xC5, 0x7F, 0xE1, 0x21, 0x41, 0x89, 0x00, 0x86, 0x62, 0x6F, 0xC2, 0xC9, 0x73],
        "%APPDATA%\\Microsoft\\Windows\\Network Shortcuts",
    ),
    // FOLDERID_NetworkFolder {D20BEEC4-5CA8-4905-AE3B-BF251EA09B53}
    (
        "NetworkFolder",
        [0xC4, 0xEE, 0x0B, 0xD2, 0xA8, 0x5C, 0x05, 0x49, 0xAE, 0x3B, 0xBF, 0x25, 0x1E, 0xA0, 0x9B, 0x53],
        "",
    ),
    // FOLDERID_Objects3D {31C0DD25-9439-4F12-BF41-7FF4EDA38722}
    (
        "Objects3D",
        [0x25, 0xDD, 0xC0, 0x31, 0x39, 0x94, 0x12, 0x4F, 0xBF, 0x41, 0x7F, 0xF4, 0xED, 0xA3, 0x87, 0x22],
        "%USERPROFILE%\\3D Objects",
    ),
    // FOLDERID_OriginalImages {2C36C0AA-5812-4B87-BFD0-4CD0DFB19B39}
    (
        "OriginalImages",
        [0xAA, 0xC0, 0x36, 0x2C, 0x12, 0x58, 0x87, 0x4B, 0xBF, 0xD0, 0x4C, 0xD0, 0xDF, 0xB1, 0x9B, 0x39],
        "%LOCALAPPDATA%\\Microsoft\\Windows Photo Gallery\\Original Images",
    ),
    // FOLDERID_PhotoAlbums {69D2CF90-FC33-4FB7-9A0C-EBB0F0FCB43C}
    (
        "PhotoAlbums",
        [0x90, 0xCF, 0xD2, 0x69, 0x33, 0xFC, 0xB7, 0x4F, 0x9A, 0x0C, 0xEB, 0xB0, 0xF0, 0xFC, 0xB4, 0x3C],
        "%USERPROFILE%\\Pictures\\Slide Shows",
    ),
    // FOLDERID_Pictures {33E28130-4E1E-4676-835A-98395C3BC3BB}
    (
        "Pictures",
        [0x30, 0x81, 0xE2, 0x33, 0x1E, 0x4E, 0x76, 0x46, 0x83, 0x5A, 0x98, 0x39, 0x5C, 0x3B, 0xC3, 0xBB],
        "%USERPROFILE%\\Pictures",
    ),
    // FOLDERID_PicturesLibrary {A990AE9F-A03B-4E80-94BC-9912D7504104}
    (
        "PicturesLibrary",
        [0x9F, 0xAE, 0x90, 0xA9, 0x3B, 0xA0, 0x80, 0x4E, 0x94, 0xBC, 0x99, 0x12, 0xD7, 0x50, 0x41, 0x04],
        "%APPDATA%\\Microsoft\\Windows\\Libraries\\Pictures.library-ms",
    ),
    // FOLDERID_Playlists {DE92C1C7-837F-4F69-A3BB-86E631204A23}
    (
        "Playlists",
        [0xC7, 0xC1, 0x92, 0xDE, 0x7F, 0x83, 0x69, 0x4F, 0xA3, 0xBB, 0x86, 0xE6, 0x31, 0x20, 0x4A, 0x23],
        "%USERPROFILE%\\Music\\Playlists",
    ),
    // FOLDERID_PrintHood {9274BD8D-CFD1-41C3-B35E-B13F55A758F4}
    (
        "PrintHood",
        [0x8D, 0xBD, 0x74, 0x92, 0xD1, 0xCF, 0xC3, 0x41, 0xB3, 0x5E, 0xB1, 0x3F, 0x55, 0xA7, 0x58, 0xF4],
        "%APPDATA%\\Microsoft\\Windows\\Printer Shortcuts",
    ),
    // FOLDERID_PrintersFolder {76FC4E2D-D6AD-4519-A663-37BD56068185}
    (
        "PrintersFolder",
        [0x2D, 0x4E, 0xFC, 0x76, 0xAD, 0xD6, 0x19, 0x45, 0xA6, 0x63, 0x37, 0xBD, 0x56, 0x06, 0x81, 0x85],
        "",
    ),
    // FOLDERID_Profile {5E6C858F-0E22-4760-9AFE-EA3317B67173}
    (
        "Profile",
        [0x8F, 0x85, 0x6C, 0x5E, 0x22, 0x0E, 0x60, 0x47, 0x9A, 0xFE, 0xEA, 0x33, 0x17, 0xB6, 0x71, 0x73],
        "%USERPROFILE%",
    ),
    // FOLDERID_ProgramData {62AB5D82-FDC1-4DC3-A9DD-070D1D495D97}
    (
        "ProgramData",
        [0x82, 0x5D, 0xAB, 0x62, 0xC1, 0xFD, 0xC3, 0x4D, 0xA9, 0xDD, 0x07, 0x0D, 0x1D, 0x49, 0x5D, 0x97],
        "%ALLUSERSPROFILE%",
    ),
    // FOLDERID_ProgramFiles {905E63B6-C1BF-494E-B29C-65B732D3D21A}
    (
        "ProgramFiles",
        [0xB6, 0x63, 0x5E, 0x90, 0xBF, 0xC1, 0x4E, 0x49, 0xB2, 0x9C, 0x65, 0xB7, 0x32, 0xD3, 0xD2, 0x1A],
        "%ProgramFiles%",
    ),
    // FOLDERID_ProgramFilesCommon {F7F1ED05-9F6D-47A2-AAAE-29D317C6F066}
    (
        "ProgramFilesCommon",
        [0x05, 0xED, 0xF1, 0xF7, 0x6D, 0x9F, 0xA2, 0x47, 0xAA, 0xAE, 0x29, 0xD3, 0x17, 0xC6, 0xF0, 0x66],
        "%CommonProgramFiles%",
    ),
    // FOLDERID_ProgramFilesCommonX64 {6365D5A7-0F0D-45E5-87F6-0DA56B6A4F7D}
    (
        "ProgramFilesCommonX64",
        [0xA7, 0xD5, 0x65, 0x63, 0x0D, 0x0F, 0xE5, 0x45, 0x87, 0xF6, 0x0D, 0xA5, 0x6B, 0x6A, 0x4F, 0x7D],
        "%CommonProgramFiles%",
    ),
    // FOLDERID_ProgramFilesCommonX86 {DE974D24-D9C6-4D3E-BF91-F4455120B917}
    (
        "ProgramFilesCommonX86",
        [0x24, 0x4D, 0x97, 0xDE, 0xC6, 0xD9, 0x3E, 0x4D, 0xBF, 0x91, 0xF4, 0x45, 0x51, 0x20, 0xB9, 0x17],
        "%CommonProgramFiles(x86)%",
    ),
    // FOLDERID_ProgramFilesX64 {6D809377-6AF0-444B-8957-A3773F02200E}
    (
        "ProgramFilesX64",
        [0x77, 0x93, 0x80, 0x6D, 0xF0, 0x6A, 0x4B, 0x44, 0x89, 0x57, 0xA3, 0x77, 0x3F, 0x02, 0x20, 0x0E],
        "%ProgramFiles%",
    ),
    // FOLDERID_ProgramFilesX86 {7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}
    (
        "ProgramFilesX86",
        [0xEF, 0x40, 0x5A, 0x7C, 0xFB, 0xA0, 0xFC, 0x4B, 0x87, 0x4A, 0xC0, 0xF2, 0xE0, 0xB9, 0xFA, 0x8E],
        "%ProgramFiles(x86)%",
    ),
    // FOLDERID_Programs {A77F5D77-2E2B-44C3-A6A2-ABA601054A51}
    (
        "Programs",
        [0x77, 0x5D, 0x7F, 0xA7, 0x2B, 0x2E, 0xC3, 0x44, 0xA6, 0xA2, 0xAB, 0xA6, 0x01, 0x05, 0x4A, 0x51],
        "%APPDATA%\\Microsoft\\Windows\\Start Menu\\Programs",
    ),
    // FOLDERID_Public {DFDF76A2-C82A-4D63-906A-5644AC457385}
    (
        "Public",
        [0xA2, 0x76, 0xDF, 0xDF, 0x2A, 0xC8, 0x63, 0x4D, 0x90, 0x6A, 0x56, 0x44, 0xAC, 0x45, 0x73, 0x85],
        "%PUBLIC%",
    ),
    // FOLDERID_PublicDesktop {C4AA340D-F20F-4863-AFEF-F87EF2E6BA25}
    (
        "PublicDesktop",
        [0x0D, 0x34, 0xAA, 0xC4, 0x0F, 0xF2, 0x63, 0x48, 0xAF, 0xEF, 0xF8, 0x7E, 0xF2, 0xE6, 0xBA, 0x25],
        "%PUBLIC%\\Desktop",
    ),
    // FOLDERID_PublicDocuments {ED4824AF-DCE4-45A8-81E2-FC7965083634}
    (
        "PublicDocuments",
        [0xAF, 0x24, 0x48, 0xED, 0xE4, 0xDC, 0xA8, 0x45, 0x81, 0xE2, 0xFC, 0x79, 0x65, 0x08, 0x36, 0x34],
        "%PUBLIC%\\Documents",
    ),
    // FOLDERID_PublicDownloads {3D644C9B-1FB8-4F30-9B45-F670235F79C0}
    (
        "PublicDownloads",
        [0x9B, 0x4C, 0x64, 0x3D, 0xB8, 0x1F, 0x30, 0x4F, 0x9B, 0x45, 0xF6, 0x70, 0x23, 0x5F, 0x79, 0xC0],
        "%PUBLIC%\\Downloads",
    ),
    // FOLDERID_PublicGameTasks {DEBF2536-E1A8-4C59-B6A2-414586476AEA}
    (
        "PublicGameTasks",
        [0x36, 0x25, 0xBF, 0xDE, 0xA8, 0xE1, 0x59, 0x4C, 0xB6, 0xA2, 0x41, 0x45, 0x86, 0x47, 0x6A, 0xEA],
        "%ALLUSERSPROFILE%\\Microsoft\\Windows\\GameExplorer",
    ),
    // FOLDERID_PublicLibraries {48DAF80B-E6CF-4F4E-B800-0E69D84EE384}
    (
        "PublicLibraries",
        [0x0B, 0xF8, 0xDA, 0x48, 0xCF, 0xE6, 0x4E, 0x4F, 0xB8, 0x00, 0x0E, 0x69, 0xD8, 0x4E, 0xE3, 0x84],
        "%ALLUSERSPROFILE%\\Microsoft\\Windows\\Libraries",
    ),
    // FOLDERID_PublicMusic {3214FAB5-9757-4298-BB61-92A9DEAA44FF}
    (
        "PublicMusic",
        [0xB5, 0xFA, 0x14, 0x32, 0x57, 0x97, 0x98, 0x42, 0xBB, 0x61, 0x92, 0xA9, 0xDE, 0xAA, 0x44, 0xFF],
        "%PUBLIC%\\Music",
    ),
    // FOLDERID_PublicPictures {B6EBFB86-6907-413C-9AF7-4FC2ABF07CC5}
    (
        "PublicPictures",
        [0x86, 0xFB, 0xEB, 0xB6, 0x07, 0x69, 0x3C, 0x41, 0x9A, 0xF7, 0x4F, 0xC2, 0xAB, 0xF0, 0x7C, 0xC5],
        "%PUBLIC%\\Pictures",
    ),
    // FOLDERID_PublicRingtones {E555AB60-153B-4D17-9F04-A5FE99FC15EC}
    (
        "PublicRingtones",
        [0x60, 0xAB, 0x55, 0xE5, 0x3B, 0x15, 0x17, 0x4D, 0x9F, 0x04, 0xA5, 0xFE, 0x99, 0xFC, 0x15, 0xEC],
        "%ALLUSERSPROFILE%\\Microsoft\\Windows\\Ringtones",
    ),
    // FOLDERID_PublicUserTiles {0482AF6C-08F1-4C34-8C90-E17EC98B1E17}
    (
        "PublicUserTiles",
        [0x6C, 0xAF, 0x82, 0x04, 0xF1, 0x08, 0x34, 0x4C, 0x8C, 0x90, 0xE1, 0x7E, 0xC9, 0x8B, 0x1E, 0x17],
        "%PUBLIC%\\AccountPictures",
    ),
    // FOLDERID_PublicVideos {2400183A-6185-49FB-A2D8-4A392A602BA3}
    (
        "PublicVideos",
        [0x3A, 0x18, 0x00, 0x24, 0x85, 0x61, 0xFB, 0x49, 0xA2, 0xD8, 0x4A, 0x39, 0x2A, 0x60, 0x2B, 0xA3],
        "%PUBLIC%\\Videos",
    ),
    // FOLDERID_QuickLaunch {52A4F021-7B75-48A9-9F6B-4B87A210BC8F}
    (
        "QuickLaunch",
        [0x21, 0xF0, 0xA4, 0x52, 0x75, 0x7B, 0xA9, 0x48, 0x9F, 0x6B, 0x4B, 0x87, 0xA2, 0x10, 0xBC, 0x8F],
        "%APPDATA%\\Microsoft\\Internet Explorer\\Quick Launch",
    ),
    // FOLDERID_Recent {AE50C081-EBD2-438A-8655-8A092E34987A}
    (
        "Recent",
        [0x81, 0xC0, 0x50, 0xAE, 0xD2, 0xEB, 0x8A, 0x43, 0x86, 0x55, 0x8A, 0x09, 0x2E, 0x34, 0x98, 0x7A],
        "%APPDATA%\\Microsoft\\Windows\\Recent",
    ),
    // FOLDERID_RecordedTVLibrary {1A6FDBA2-F42D-4358-A798-B74D745926C5}
    (
        "RecordedTVLibrary",
        [0xA2, 0xDB, 0x6F, 0x1A, 0x2D, 0xF4, 0x58, 0x43, 0xA7, 0x98, 0xB7, 0x4D, 0x74, 0x59, 0x26, 0xC5],
        "%PUBLIC%\\RecordedTV.library-ms",
    ),
    // FOLDERID_RecycleBinFolder {B7534046-3ECB-4C18-BE4E-64CD4CB7D6AC}
    (
        "RecycleBinFolder",
        [0x46, 0x40, 0x53, 0xB7, 0xCB, 0x3E, 0x18, 0x4C, 0xBE, 0x4E, 0x64, 0xCD, 0x4C, 0xB7, 0xD6, 0xAC],
        "",
    ),
    // FOLDERID_ResourceDir {8AD10C31-2ADB-4296-A8F7-E4701232C972}
    (
        "ResourceDir",
        [0x31, 0x0C, 0xD1, 0x8A, 0xDB, 0x2A, 0x96, 0x42, 0xA8, 0xF7, 0xE4, 0x70, 0x12, 0x32, 0xC9, 0x72],
        "%windir%\\Resources",
    ),
    // FOLDERID_Ringtones {C870044B-F49E-4126-A9C3-B52A1FF411E8}
    (
        "Ringtones",
        [0x4B, 0x04, 0x70, 0xC8, 0x9E, 0xF4, 0x26, 0x41, 0xA9, 0xC3, 0xB5, 0x2A, 0x1F, 0xF4, 0x11, 0xE8],
        "%LOCALAPPDATA%\\Microsoft\\Windows\\Ringtones",
    ),
    // FOLDERID_RoamedTileImages {AAA8D5A5-F1D6-4259-BAA8-78E7EF60835E}
    (
        "RoamedTileImages",
        [0xA5, 0xD5, 0xA8, 0xAA, 0xD6, 0xF1, 0x59, 0x42, 0xBA, 0xA8, 0x78, 0xE7, 0xEF, 0x60, 0x83, 0x5E],
        "%LOCALAPPDATA%\\Microsoft\\Windows\\RoamedTileImages",
    ),
    // FOLDERID_RoamingAppData {3EB685DB-65F9-4CF6-A03A-E3EF65729F3D}
    (
        "RoamingAppData",
        [0xDB, 0x85, 0xB6, 0x3E, 0xF9, 0x65, 0xF6, 0x4C, 0xA0, 0x3A, 0xE3, 0xEF, 0x65, 0x72, 0x9F, 0x3D],
        "%APPDATA%",
    ),
    // FOLDERID_RoamingTiles {00BCFC5A-ED94-4E48-96A1-3F6217F21990}
    (
        "RoamingTiles",
        [0x5A, 0xFC, 0xBC, 0x00, 0x94, 0xED, 0x48, 0x4E, 0x96, 0xA1, 0x3F, 0x62, 0x17, 0xF2, 0x19, 0x90],
        "%LOCALAPPDATA%\\Microsoft\\Windows\\RoamingTiles",
    ),
    // FOLDERID_SampleMusic {B250C668-F57D-4EE1-A63C-290EE7D1AA1F}
    (
        "SampleMusic",
        [0x68, 0xC6, 0x50, 0xB2, 0x7D, 0xF5, 0xE1, 0x4E, 0xA6, 0x3C, 0x29, 0x0E, 0xE7, 0xD1, 0xAA, 0x1F],
        "%PUBLIC%\\Music\\Sample Music",
    ),
    // FOLDERID_SamplePictures {C4900540-2379-4C75-844B-64E6FAF8716B}
    (
        "SamplePictures",
        [0x40, 0x05, 0x90, 0xC4, 0x79, 0x23, 0x75, 0x4C, 0x84, 0x4B, 0x64, 0xE6, 0xFA, 0xF8, 0x71, 0x6B],
        "%PUBLIC%\\Pictures\\Sample Pictures",
    ),
    // FOLDERID_SamplePlaylists {15CA69B3-30EE-49C1-ACE1-6B5EC372AFB5}
    (
        "SamplePlaylists",
        [0xB3, 0x69, 0xCA, 0x15, 0xEE, 0x30, 0xC1, 0x49, 0xAC, 0xE1, 0x6B, 0x5E, 0xC3, 0x72, 0xAF, 0xB5],
        "%PUBLIC%\\Music\\Sample Playlists",
    ),
    // FOLDERID_SampleVideos {859EAD94-2E85-48AD-A71A-0969CB56A6CD}
    (
        "SampleVideos",
        [0x94, 0xAD, 0x9E, 0x85, 0x85, 0x2E, 0xAD, 0x48, 0xA7, 0x1A, 0x09, 0x69, 0xCB, 0x56, 0xA6, 0xCD],
        "%PUBLIC%\\Videos\\Sample Videos",
    ),
    // FOLDERID_SavedGames {4C5C32FF-BB9D-43B0-B5B4-2D72E54EAAA4}
    (
        "SavedGames",
        [0xFF, 0x32, 0x5C, 0x4C, 0x9D, 0xBB, 0xB0, 0x43, 0xB5, 0xB4, 0x2D, 0x72, 0xE5, 0x4E, 0xAA, 0xA4],
        "%USERPROFILE%\\Saved Games",
    ),
    // FOLDERID_SavedPictures {3B193882-D3AD-4EAB-965A-69829D1FB59F}
    (
        "SavedPictures",
        [0x82, 0x38, 0x19, 0x3B, 0xAD, 0xD3, 0xAB, 0x4E, 0x96, 0x5A, 0x69, 0x82, 0x9D, 0x1F, 0xB5, 0x9F],
        "%USERPROFILE%\\Pictures\\Saved Pictures",
    ),
    // FOLDERID_SavedPicturesLibrary {E25B5812-BE88-4BD9-94B0-29233477B6C3}
    (
        "SavedPicturesLibrary",
        [0x12, 0x58, 0x5B, 0xE2, 0x88, 0xBE, 0xD9, 0x4B, 0x94, 0xB0, 0x29, 0x23, 0x34, 0x77, 0xB6, 0xC3],
        "%APPDATA%\\Microsoft\\Windows\\Libraries\\SavedPictures.library-ms",
    ),
    // FOLDERID_SavedSearches {7D1D3A04-DEBB-4115-95CF-2F29DA2920DA}
    (
        "SavedSearches",
        [0x04, 0x3A, 0x1D, 0x7D, 0xBB, 0xDE, 0x15, 0x41, 0x95, 0xCF, 0x2F, 0x29, 0xDA, 0x29, 0x20, 0xDA],
        "%USERPROFILE%\\Searches",
    ),
    // FOLDERID_Screenshots {B7BEDE81-DF94-4682-A7D8-57A52620B86F}
    (
        "Screenshots",
        [0x81, 0xDE, 0xBE, 0xB7, 0x94, 0xDF, 0x82, 0x46, 0xA7, 0xD8, 0x57, 0xA5, 0x26, 0x20, 0xB8, 0x6F],
        "%USERPROFILE%\\Pictures\\Screenshots",
    ),
    // FOLDERID_SearchHistory {0D4C3DB6-03A3-462F-A0E6-08924C41B5D4}
    (
        "SearchHistory",
        [0xB6, 0x3D, 0x4C, 0x0D, 0xA3, 0x03, 0x2F, 0x46, 0xA0, 0xE6, 0x08, 0x92, 0x4C, 0x41, 0xB5, 0xD4],
        "%LOCALAPPDATA%\\Microsoft\\Windows\\ConnectedSearch\\History",
    ),
    // FOLDERID_SearchHome {190337D1-B8CA-4121-A639-6D472D16972A}
    (
        "SearchHome",
        [0xD1, 0x37, 0x03, 0x19, 0xCA, 0xB8, 0x21, 0x41, 0xA6, 0x39, 0x6D, 0x47, 0x2D, 0x16, 0x97, 0x2A],
        "",
    ),
    // FOLDERID_SearchTemplates {7E636BFE-DFA9-4D5E-B456-D7B39851D8A9}
    (
        "SearchTemplates",
        [0xFE, 0x6B, 0x63, 0x7E, 0xA9, 0xDF, 0x5E, 0x4D, 0xB4, 0x56, 0xD7, 0xB3, 0x98, 0x51, 0xD8, 0xA9],
        "%LOCALAPPDATA%\\Microsoft\\Windows\\ConnectedSearch\\Templates",
    ),
    // FOLDERID_SendTo {8983036C-27C0-404B-8F08-102D10DCFD74}
    (
        "SendTo",
        [0x6C, 0x03, 0x83, 0x89, 0xC0, 0x27, 0x4B, 0x40, 0x8F, 0x08, 0x10, 0x2D, 0x10, 0xDC, 0xFD, 0x74],
        "%APPDATA%\\Microsoft\\Windows\\SendTo",
    ),
    // FOLDERID_SidebarDefaultParts {7B396E54-9EC5-4300-BE0A-2482EBAE1A26}
    (
        "SidebarDefaultParts",
        [0x54, 0x6E, 0x39, 0x7B, 0xC5, 0x9E, 0x00, 0x43, 0xBE, 0x0A, 0x24, 0x82, 0xEB, 0xAE, 0x1A, 0x26],
        "%ProgramFiles%\\Windows Sidebar\\Gadgets",
    ),
    // FOLDERID_SidebarParts {A75D362E-50FC-4FB7-AC2C-A8BEAA314493}
    (
        "SidebarParts",
        [0x2E, 0x36, 0x5D, 0xA7, 0xFC, 0x50, 0xB7, 0x4F, 0xAC, 0x2C, 0xA8, 0xBE, 0xAA, 0x31, 0x44, 0x93],
        "%LOCALAPPDATA%\\Microsoft\\Windows Sidebar\\Gadgets",
    ),
    // FOLDERID_SkyDrive {A52BBA46-E9E1-435F-B3D9-28DAA648C0F6}
    (
        "SkyDrive",
        [0x46, 0xBA, 0x2B, 0xA5, 0xE1, 0xE9, 0x5F, 0x43, 0xB3, 0xD9, 0x28, 0xDA, 0xA6, 0x48, 0xC0, 0xF6],
        "%USERPROFILE%\\OneDrive",
    ),
    // FOLDERID_SkyDriveCameraRoll {767E6811-49CB-4273-87C2-20F355E1085B}
    (
        "SkyDriveCameraRoll",
        [0x11, 0x68, 0x7E, 0x76, 0xCB, 0x49, 0x73, 0x42, 0x87, 0xC2, 0x20, 0xF3, 0x55, 0xE1, 0x08, 0x5B],
        "%USERPROFILE%\\OneDrive\\Pictures\\Camera Roll",
    ),
    // FOLDERID_SkyDriveDocuments {24D89E24-2F19-4534-9DDE-6A6671FBB8FE}
    (
        "SkyDriveDocuments",
        [0x24, 0x9E, 0xD8, 0x24, 0x19, 0x2F, 0x34, 0x45, 0x9D, 0xDE, 0x6A, 0x66, 0x71, 0xFB, 0xB8, 0xFE],
        "%USERPROFILE%\\OneDrive\\Documents",
    ),
    // FOLDERID_SkyDrivePictures {339719B5-8C47-4894-94C2-D8F77ADD44A6}
    (
        "SkyDrivePictures",
        [0xB5, 0x19, 0x97, 0x33, 0x47, 0x8C, 0x94, 0x48, 0x94, 0xC2, 0xD8, 0xF7, 0x7A, 0xDD, 0x44, 0xA6],
        "%USERPROFILE%\\OneDrive\\Pictures",
    ),
    // FOLDERID_StartMenu {625B53C3-AB48-4EC1-BA1F-A1EF4146FC19}
    (
        "StartMenu",
        [0xC3, 0x53, 0x5B, 0x62, 0x48, 0xAB, 0xC1, 0x4E, 0xBA, 0x1F, 0xA1, 0xEF, 0x41, 0x46, 0xFC, 0x19],
        "%APPDATA%\\Microsoft\\Windows\\Start Menu",
    ),
    // FOLDERID_Startup {B97D20BB-F46A-4C97-BA10-5E3608430854}
    (
        "Startup",
        [0xBB, 0x20, 0x7D, 0xB9, 0x6A, 0xF4, 0x97, 0x4C, 0xBA, 0x10, 0x5E, 0x36, 0x08, 0x43, 0x08, 0x54],
        "%APPDATA%\\Microsoft\\Windows\\Start Menu\\Programs\\StartUp",
    ),
    // FOLDERID_SyncManagerFolder {43668BF8-C14E-49B2-97C9-747784D784B7}
    (
        "SyncManagerFolder",
        [0xF8, 0x8B, 0x66, 0x43, 0x4E, 0xC1, 0xB2, 0x49, 0x97, 0xC9, 0x74, 0x77, 0x84, 0xD7, 0x84, 0xB7],
        "",
    ),
    // FOLDERID_SyncResultsFolder {289A9A43-BE44-4057-A41B-587A76D7E7F9}
    (
        "SyncResultsFolder",
        [0x43, 0x9A, 0x9A, 0x28, 0x44, 0xBE, 0x57, 0x40, 0xA4, 0x1B, 0x58, 0x7A, 0x76, 0xD7, 0xE7, 0xF9],
        "",
    ),
    // FOLDERID_SyncSetupFolder {0F214138-B1D3-4A90-BBA9-27CBC0C5389A}
    (
        "SyncSetupFolder",
        [0x38, 0x41, 0x21, 0x0F, 0xD3, 0xB1, 0x90, 0x4A, 0xBB, 0xA9, 0x27, 0xCB, 0xC0, 0xC5, 0x38, 0x9A],
        "",
    ),
    // FOLDERID_System {1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}
    (
        "System",
        [0x77, 0x4E, 0xC1, 0x1A, 0xE7, 0x02, 0x5D, 0x4E, 0xB7, 0x44, 0x2E, 0xB1, 0xAE, 0x51, 0x98, 0xB7],
        "%windir%\\system32",
    ),
    // FOLDERID_SystemX86 {D65231B0-B2F1-4857-A4CE-A8E7C6EA7D27}
    (
        "SystemX86",
        [0xB0, 0x31, 0x52, 0xD6, 0xF1, 0xB2, 0x57, 0x48, 0xA4, 0xCE, 0xA8, 0xE7, 0xC6, 0xEA, 0x7D, 0x27],
        "%windir%\\SysWOW64",
    ),
    // FOLDERID_Templates {A63293E8-664E-48DB-A079-DF759E0509F7}
    (
        "Templates",
        [0xE8, 0x93, 0x32, 0xA6, 0x4E, 0x66, 0xDB, 0x48, 0xA0, 0x79, 0xDF, 0x75, 0x9E, 0x05, 0x09, 0xF7],
        "%APPDATA%\\Microsoft\\Windows\\Templates",
    ),
    // FOLDERID_UserPinned {9E3995AB-1F9C-4F13-B827-48B24B6C7174}
    (
        "UserPinned",
        [0xAB, 0x95, 0x39, 0x9E, 0x9C, 0x1F, 0x13, 0x4F, 0xB8, 0x27, 0x48, 0xB2, 0x4B, 0x6C, 0x71, 0x74],
        "%APPDATA%\\Microsoft\\Internet Explorer\\Quick Launch\\User Pinned",
    ),
    // FOLDERID_UserProfiles {0762D272-C50A-4BB0-A382-697DCD729B80}
    (
        "UserProfiles",
        [0x72, 0xD2, 0x62, 0x07, 0x0A, 0xC5, 0xB0, 0x4B, 0xA3, 0x82, 0x69, 0x7D, 0xCD, 0x72, 0x9B, 0x80],
        "%SystemDrive%\\Users",
    ),
    // FOLDERID_UserProgramFiles {5CD7AEE2-2219-4A67-B85D-6C9CE15660CB}
    (
        "UserProgramFiles",
        [0xE2, 0xAE, 0xD7, 0x5C, 0x19, 0x22, 0x67, 0x4A, 0xB8, 0x5D, 0x6C, 0x9C, 0xE1, 0x56, 0x60, 0xCB],
        "%LOCALAPPDATA%\\Programs",
    ),
    // FOLDERID_UserProgramFilesCommon {BCBD3057-CA5C-4622-B42D-BC56DB0AE516}
    (
        "UserProgramFilesCommon",
        [0x57, 0x30, 0xBD, 0xBC, 0x5C, 0xCA, 0x22, 0x46, 0xB4, 0x2D, 0xBC, 0x56, 0xDB, 0x0A, 0xE5, 0x16],
        "%LOCALAPPDATA%\\Programs\\Common",
    ),
    // FOLDERID_UsersFiles {F3CE0F7C-4901-4ACC-8648-D5D44B04EF8F}
    (
        "UsersFiles",
        [0x7C, 0x0F, 0xCE, 0xF3, 0x01, 0x49, 0xCC, 0x4A, 0x86, 0x48, 0xD5, 0xD4, 0x4B, 0x04, 0xEF, 0x8F],
        "",
    ),
    // FOLDERID_UsersLibraries {A302545D-DEFF-464B-ABE8-61C8648D939B}
    (
        "UsersLibraries",
        [0x5D, 0x54, 0x02, 0xA3, 0xFF, 0xDE, 0x4B, 0x46, 0xAB, 0xE8, 0x61, 0xC8, 0x64, 0x8D, 0x93, 0x9B],
        "",
    ),
    // FOLDERID_Videos {18989B1D-99B5-455B-841C-AB7C74E4DDFC}
    (
        "Videos",
        [0x1D, 0x9B, 0x98, 0x18, 0xB5, 0x99, 0x5B, 0x45, 0x84, 0x1C, 0xAB, 0x7C, 0x74, 0xE4, 0xDD, 0xFC],
        "%USERPROFILE%\\Videos",
    ),
    // FOLDERID_VideosLibrary {491E922F-5643-4AF4-A7EB-4E7A138D8174}
    (
        "VideosLibrary",
        [0x2F, 0x92, 0x1E, 0x49, 0x43, 0x56, 0xF4, 0x4A, 0xA7, 0xEB, 0x4E, 0x7A, 0x13, 0x8D, 0x81, 0x74],
        "%APPDATA%\\Microsoft\\Windows\\Libraries\\Videos.library-ms",
    ),
    // FOLDERID_Windows {F38BF404-1D43-42F2-9305-67DE0B28FC23}
    (
        "Windows",
        [0x04, 0xF4, 0x8B, 0xF3, 0x43, 0x1D, 0xF2, 0x42, 0x93, 0x05, 0x67, 0xDE, 0x0B, 0x28, 0xFC, 0x23],
        "%windir%",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::collections::BTreeSet;
    use std::string::String;

    /// A GUID's text (`XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX`) as the bytes it has in memory:
    /// the first three fields little-endian, the last eight bytes as written.
    fn guid(text: &str) -> [u8; 16] {
        let hex: std::vec::Vec<u8> = text
            .replace('-', "")
            .as_bytes()
            .chunks(2)
            .map(|c| u8::from_str_radix(core::str::from_utf8(c).unwrap(), 16).unwrap())
            .collect();
        let mut g = [0u8; 16];
        g[..4].copy_from_slice(&[hex[3], hex[2], hex[1], hex[0]]);
        g[4..6].copy_from_slice(&[hex[5], hex[4]]);
        g[6..8].copy_from_slice(&[hex[7], hex[6]]);
        g[8..].copy_from_slice(&hex[8..]);
        g
    }

    fn env(name: &str) -> Option<Vec<u16>> {
        let v = match name {
            "LOCALAPPDATA" => "C:\\Users\\poly\\AppData\\Local",
            "APPDATA" => "C:\\Users\\poly\\AppData\\Roaming",
            "USERPROFILE" => "C:\\Users\\poly",
            "windir" => "C:\\Windows",
            "ProgramFiles" => "C:\\Program Files",
            "ProgramFiles(x86)" => "C:\\Program Files (x86)",
            "SystemDrive" => "C:",
            "PUBLIC" => "C:\\Users\\Public\\",
            _ => return None,
        };
        Some(v.encode_utf16().collect())
    }

    fn text(id: &str) -> Result<String, u32> {
        known_folder_path(&guid(id), env).map(|p| String::from_utf16(&p).unwrap())
    }

    #[test]
    fn the_folders_chrome_and_installers_ask_for() {
        // FOLDERID_LocalAppData, RoamingAppData, Profile, Windows, System, ProgramFiles.
        assert_eq!(text("F1B32785-6FBA-4FCF-9D55-7B8E7F157091").as_deref(), Ok("C:\\Users\\poly\\AppData\\Local"));
        assert_eq!(text("3EB685DB-65F9-4CF6-A03A-E3EF65729F3D").as_deref(), Ok("C:\\Users\\poly\\AppData\\Roaming"));
        assert_eq!(text("5E6C858F-0E22-4760-9AFE-EA3317B67173").as_deref(), Ok("C:\\Users\\poly"));
        assert_eq!(text("F38BF404-1D43-42F2-9305-67DE0B28FC23").as_deref(), Ok("C:\\Windows"));
        assert_eq!(text("1AC14E77-02E7-4E5D-B744-2EB1AE5198B7").as_deref(), Ok("C:\\Windows\\system32"));
        assert_eq!(text("905E63B6-C1BF-494E-B29C-65B732D3D21A").as_deref(), Ok("C:\\Program Files"));
        assert_eq!(text("7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E").as_deref(), Ok("C:\\Program Files (x86)"));
        // Folders below a variable.
        assert_eq!(text("374DE290-123F-4565-9164-39C4925E467B").as_deref(), Ok("C:\\Users\\poly\\Downloads"));
        assert_eq!(text("A520A1A4-1780-4FF6-BD18-167343C5AF16").as_deref(), Ok("C:\\Users\\poly\\AppData\\LocalLow"));
        assert_eq!(text("0762D272-C50A-4BB0-A382-697DCD729B80").as_deref(), Ok("C:\\Users"));
        // A variable's trailing backslash is not doubled.
        assert_eq!(text("C4AA340D-F20F-4863-AFEF-F87EF2E6BA25").as_deref(), Ok("C:\\Users\\Public\\Desktop"));
        assert_eq!(text("DFDF76A2-C82A-4D63-906A-5644AC457385").as_deref(), Ok("C:\\Users\\Public"));
    }

    #[test]
    fn folders_without_a_path_unknown_ids_and_unset_variables() {
        // FOLDERID_ComputerFolder is virtual: no path.
        assert_eq!(text("0AC0837C-BBF8-452A-850D-79D08E667CA7"), Err(E_FAIL));
        // Not a known folder.
        assert_eq!(text("00000000-0000-0000-0000-000000000000"), Err(E_FILE_NOT_FOUND));
        // FOLDERID_ProgramData: %ALLUSERSPROFILE% is not set in this environment.
        assert_eq!(text("62AB5D82-FDC1-4DC3-A9DD-070D1D495D97"), Err(E_PATH_NOT_FOUND));
    }

    #[test]
    fn the_default_user_has_its_own_profile_and_system_folders_stay() {
        let w = |s: &str| -> Vec<u16> { s.encode_utf16().collect() };
        let profile = w("C:\\Users\\poly");
        let moved = |p: &str| String::from_utf16(&for_default_user(&w(p), &profile)).unwrap();
        assert_eq!(moved("C:\\Users\\poly"), "C:\\Users\\Default");
        assert_eq!(moved("C:\\Users\\poly\\Downloads"), "C:\\Users\\Default\\Downloads");
        assert_eq!(moved("c:\\users\\POLY\\AppData\\Local"), "C:\\Users\\Default\\AppData\\Local");
        assert_eq!(moved("C:\\Windows"), "C:\\Windows");
        assert_eq!(moved("C:\\Users\\polyglot"), "C:\\Users\\polyglot");
        assert_eq!(moved("C:\\Users"), "C:\\Users");
        let with_slash = for_default_user(&w("C:\\Users\\poly\\x"), &w("C:\\Users\\poly\\"));
        assert_eq!(String::from_utf16(&with_slash).unwrap(), "C:\\Users\\Default\\x");
    }

    #[test]
    fn ids_and_names_are_unique_and_paths_start_with_a_variable() {
        let ids: BTreeSet<[u8; 16]> = KNOWN_FOLDERS.iter().map(|f| f.1).collect();
        let names: BTreeSet<&str> = KNOWN_FOLDERS.iter().map(|f| f.0).collect();
        assert_eq!(ids.len(), KNOWN_FOLDERS.len());
        assert_eq!(names.len(), KNOWN_FOLDERS.len());
        assert!(KNOWN_FOLDERS.len() > 100);
        for (name, _, template) in KNOWN_FOLDERS {
            if !template.is_empty() {
                let rest = template.strip_prefix('%').unwrap_or_else(|| panic!("{name}: {template}"));
                assert!(rest.contains('%'), "{name}: {template}");
                assert!(!template.ends_with('\\'), "{name}: {template}");
            }
        }
        assert_eq!(lookup(&guid("FDD39AD0-238F-46AF-ADB4-6C85480369C7")).map(|f| f.0), Some("Documents"));
        // The sheet's flags.
        for flags in [0, 0x4000, 0x8000, 0x2000, 0x1000, 0x400, 0x600, 0x100, 0x1, 0xFF] {
            assert!(flags_accepted(flags), "{flags:#x}");
        }
        for flags in [0x200, 0x8000_0000, 0x8000_0400] {
            assert!(!flags_accepted(flags), "{flags:#x}");
        }
    }
}
