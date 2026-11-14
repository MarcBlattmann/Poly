//! Tests: the object-manager name of the file a DOS path names.

use super::*;

#[test]
fn a_dos_path_names_its_volume_device_and_the_path_below_it() {
    // K32GetMappedFileNameW.yaml `name_form`: \Device\<volume>\ then the path without drive and colon.
    let t = setup();
    let name = |s: &str| {
        let w: Vec<u16> = s.encode_utf16().collect();
        t.f.sys.io.dos_file_name(&w).map(|(d, p)| (String::from_utf16_lossy(&d), String::from_utf16_lossy(&p)))
    };
    assert_eq!(name("Z:\\test-binary"), Some(("\\Device\\PolyFs0".into(), "\\test-binary".into())));
    assert_eq!(
        name("c:\\Windows\\System32\\ntdll.dll"),
        Some(("\\Device\\PolyFs1".into(), "\\Windows\\System32\\ntdll.dll".into()))
    );
    assert_eq!(name("\\??\\C:/Windows/x.exe"), Some(("\\Device\\PolyFs1".into(), "\\Windows\\x.exe".into())));
    assert_eq!(name("Q:\\none.exe"), None);
    assert_eq!(name("C:none.exe"), None);
    assert_eq!(name("\\\\server\\share\\a.exe"), None);
}
