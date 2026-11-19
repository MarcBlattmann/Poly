//! The in-process classes Poly's own DLLs serve (ADR 0223).
//!
//! On Windows `CoCreateInstance` finds an in-process class through the registry
//! (`HKEY_CLASSES_ROOT\CLSID\{clsid}\InprocServer32`), loads that DLL and asks its
//! `DllGetClassObject` for the class object (Learn: "CoGetClassObject", "DllGetClassObject",
//! "InprocServer32"). Poly's system DLLs are part of the system, not installed programs, so their
//! classes are listed here instead of in a registry hive: `ole32` loads the DLL named for a class and
//! goes the same way through its `DllGetClassObject` and `IClassFactory::CreateInstance`.
//!
//! The class ids are the SDK headers' (`shobjidl_core.h`, `shobjidl.h`, `netfw.h`).

use crate::guid::Guid;

/// `CLSID_FileOperation` (`shobjidl_core.h`): {3AD05575-8857-4850-9277-11B85BDB8E09}.
pub const CLSID_FILE_OPERATION: Guid =
    Guid { data1: 0x3AD0_5575, data2: 0x8857, data3: 0x4850, data4: [0x92, 0x77, 0x11, 0xB8, 0x5B, 0xDB, 0x8E, 0x09] };
/// `CLSID_TaskbarList` (`shobjidl_core.h`): {56FDF344-FD6D-11D0-958A-006097C9A090}.
pub const CLSID_TASKBAR_LIST: Guid =
    Guid { data1: 0x56FD_F344, data2: 0xFD6D, data3: 0x11D0, data4: [0x95, 0x8A, 0x00, 0x60, 0x97, 0xC9, 0xA0, 0x90] };
/// `CLSID_DestinationList` (`shobjidl_core.h`): {77F10CF0-3DB5-4966-B520-B7C54FD35ED6}.
pub const CLSID_DESTINATION_LIST: Guid =
    Guid { data1: 0x77F1_0CF0, data2: 0x3DB5, data3: 0x4966, data4: [0xB5, 0x20, 0xB7, 0xC5, 0x4F, 0xD3, 0x5E, 0xD6] };
/// `CLSID_FileSaveDialog` (`shobjidl_core.h`): {C0B4E2F3-BA21-4773-8DBA-335EC946EB8B}.
pub const CLSID_FILE_SAVE_DIALOG: Guid =
    Guid { data1: 0xC0B4_E2F3, data2: 0xBA21, data3: 0x4773, data4: [0x8D, 0xBA, 0x33, 0x5E, 0xC9, 0x46, 0xEB, 0x8B] };
/// `CLSID_NetFwPolicy2` (`netfw.h`): {E2B3C97F-6AE1-41AC-817A-F6F92166D7DD}.
pub const CLSID_NET_FW_POLICY2: Guid =
    Guid { data1: 0xE2B3_C97F, data2: 0x6AE1, data3: 0x41AC, data4: [0x81, 0x7A, 0xF6, 0xF9, 0x21, 0x66, 0xD7, 0xDD] };
/// `CLSID_NetFwRule` (`netfw.h`): {2C5BC43E-3369-4C33-AB0C-BE9469677AF4}.
pub const CLSID_NET_FW_RULE: Guid =
    Guid { data1: 0x2C5B_C43E, data2: 0x3369, data3: 0x4C33, data4: [0xAB, 0x0C, 0xBE, 0x94, 0x69, 0x67, 0x7A, 0xF4] };

/// `IID_IClassFactory` (`unknwn.h`): {00000001-0000-0000-C000-000000000046}.
pub const IID_ICLASS_FACTORY: Guid = Guid { data1: 1, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
/// `IID_IUnknown` (`unknwn.h`): {00000000-0000-0000-C000-000000000046}.
pub const IID_IUNKNOWN: Guid = Guid { data1: 0, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };

/// `CLSCTX_INPROC_SERVER` (`wtypesbase.h`).
pub const CLSCTX_INPROC_SERVER: u32 = 0x1;

/// The classes and the DLL (its file name, NUL-terminated UTF-8) that serves each.
const CLASSES: &[(Guid, &str)] = &[
    (CLSID_FILE_OPERATION, "shell32.dll\0"),
    (CLSID_TASKBAR_LIST, "shell32.dll\0"),
    (CLSID_DESTINATION_LIST, "shell32.dll\0"),
    (CLSID_FILE_SAVE_DIALOG, "comdlg32.dll\0"),
    // The Windows Firewall's policy and rule objects.
    (CLSID_NET_FW_POLICY2, "hnetcfg.dll\0"),
    (CLSID_NET_FW_RULE, "hnetcfg.dll\0"),
];

/// The DLL that serves `clsid` in the process, if Poly has the class.
pub fn server_of(clsid: &Guid) -> Option<&'static str> {
    CLASSES.iter().find(|(c, _)| c == clsid).map(|&(_, dll)| dll)
}

/// Whether a `CLSCTX` value lets an in-process server run the class. Every Poly class is one, so
/// a context without `CLSCTX_INPROC_SERVER` finds no class (`REGDB_E_CLASSNOTREG`), as on Windows
/// for a class registered only as `InprocServer32`.
pub fn context_allows_inproc(context: u32) -> bool {
    context & CLSCTX_INPROC_SERVER != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shell_classes_are_served_by_shell32_and_the_dialogs_by_comdlg32() {
        assert_eq!(server_of(&CLSID_FILE_OPERATION), Some("shell32.dll\0"));
        assert_eq!(server_of(&CLSID_TASKBAR_LIST), Some("shell32.dll\0"));
        assert_eq!(server_of(&CLSID_DESTINATION_LIST), Some("shell32.dll\0"));
        assert_eq!(server_of(&CLSID_FILE_SAVE_DIALOG), Some("comdlg32.dll\0"));
        assert_eq!(server_of(&CLSID_NET_FW_POLICY2), Some("hnetcfg.dll\0"));
        assert_eq!(server_of(&CLSID_NET_FW_RULE), Some("hnetcfg.dll\0"));
        assert_eq!(server_of(&crate::git::CLSID_BACKGROUND_COPY_MANAGER), None);
        assert_eq!(server_of(&IID_IUNKNOWN), None);
    }

    #[test]
    fn the_class_ids_read_as_the_headers_write_them() {
        let text = |g: &Guid| alloc::string::String::from_utf16(&g.to_text()).unwrap();
        assert_eq!(text(&CLSID_FILE_OPERATION), "{3AD05575-8857-4850-9277-11B85BDB8E09}");
        assert_eq!(text(&CLSID_TASKBAR_LIST), "{56FDF344-FD6D-11D0-958A-006097C9A090}");
        assert_eq!(text(&CLSID_DESTINATION_LIST), "{77F10CF0-3DB5-4966-B520-B7C54FD35ED6}");
        assert_eq!(text(&CLSID_FILE_SAVE_DIALOG), "{C0B4E2F3-BA21-4773-8DBA-335EC946EB8B}");
    }

    #[test]
    fn only_an_inproc_context_finds_the_classes() {
        assert!(context_allows_inproc(CLSCTX_INPROC_SERVER));
        assert!(context_allows_inproc(0x17)); // CLSCTX_ALL
        assert!(!context_allows_inproc(0x4)); // CLSCTX_LOCAL_SERVER
    }
}
