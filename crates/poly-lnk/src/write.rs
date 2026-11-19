//! Writing a `.lnk` file: what `IPersistFile::Save` writes (the sheet `ShellLink.IPersistFile.yaml`:
//! `save_empty`, `header`, `idlist`, `linkinfo`, `strings`, `expansion`, `extra_blocks`), always
//! as a Unicode file (MS-SHLLINK 2).

use crate::idlist::{self, Component, guid_bytes};
use crate::read::{HEADER_SIZE, block, flags};
use crate::{CLSID_SHELL_LINK, Guid, Link, System, Text, file_show_command, text, volume_guid};
use alloc::vec::Vec;

/// A known folder the special-folder blocks can name: its `CSIDL` (`shlobj_core.h`), its
/// `KNOWNFOLDERID` (`KnownFolders.h`) and its path, from the environment.
struct Folder {
    csidl: u32,
    id: Guid,
    template: &'static str,
}

const fn g(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Guid {
    Guid { data1, data2, data3, data4 }
}

/// The folders a target is placed under (the deepest one that holds it wins).
const FOLDERS: &[Folder] = &[
    // CSIDL_WINDOWS, FOLDERID_Windows {F38BF404-1D43-42F2-9305-67DE0B28FC23}
    Folder {
        csidl: 0x24,
        id: g(0xF38B_F404, 0x1D43, 0x42F2, [0x93, 0x05, 0x67, 0xDE, 0x0B, 0x28, 0xFC, 0x23]),
        template: "%SystemRoot%",
    },
    // CSIDL_SYSTEM, FOLDERID_System {1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}
    Folder {
        csidl: 0x25,
        id: g(0x1AC1_4E77, 0x02E7, 0x4E5D, [0xB7, 0x44, 0x2E, 0xB1, 0xAE, 0x51, 0x98, 0xB7]),
        template: "%SystemRoot%\\System32",
    },
    // CSIDL_PROGRAM_FILES, FOLDERID_ProgramFiles {905E63B6-C1BF-494E-B29C-65B732D3D21A}
    Folder {
        csidl: 0x26,
        id: g(0x905E_63B6, 0xC1BF, 0x494E, [0xB2, 0x9C, 0x65, 0xB7, 0x32, 0xD3, 0xD2, 0x1A]),
        template: "%ProgramFiles%",
    },
    // CSIDL_PROGRAM_FILESX86, FOLDERID_ProgramFilesX86 {7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}
    Folder {
        csidl: 0x2A,
        id: g(0x7C5A_40EF, 0xA0FB, 0x4BFC, [0x87, 0x4A, 0xC0, 0xF2, 0xE0, 0xB9, 0xFA, 0x8E]),
        template: "%ProgramFiles(x86)%",
    },
    // CSIDL_COMMON_APPDATA, FOLDERID_ProgramData {62AB5D82-FDC1-4DC3-A9DD-070D1D495D97}
    Folder {
        csidl: 0x23,
        id: g(0x62AB_5D82, 0xFDC1, 0x4DC3, [0xA9, 0xDD, 0x07, 0x0D, 0x1D, 0x49, 0x5D, 0x97]),
        template: "%ProgramData%",
    },
    // CSIDL_COMMON_PROGRAMS, FOLDERID_CommonPrograms {0139D44E-6AFE-49F2-8690-3DAFCAE6FFB8}
    Folder {
        csidl: 0x17,
        id: g(0x0139_D44E, 0x6AFE, 0x49F2, [0x86, 0x90, 0x3D, 0xAF, 0xCA, 0xE6, 0xFF, 0xB8]),
        template: "%ProgramData%\\Microsoft\\Windows\\Start Menu\\Programs",
    },
    // CSIDL_COMMON_DESKTOPDIRECTORY, FOLDERID_PublicDesktop {C4AA340D-F20F-4863-AFEF-F87EF2E6BA25}
    Folder {
        csidl: 0x19,
        id: g(0xC4AA_340D, 0xF20F, 0x4863, [0xAF, 0xEF, 0xF8, 0x7E, 0xF2, 0xE6, 0xBA, 0x25]),
        template: "%PUBLIC%\\Desktop",
    },
    // CSIDL_PROFILE, FOLDERID_Profile {5E6C858F-0E22-4760-9AFE-EA3317B67173}
    Folder {
        csidl: 0x28,
        id: g(0x5E6C_858F, 0x0E22, 0x4760, [0x9A, 0xFE, 0xEA, 0x33, 0x17, 0xB6, 0x71, 0x73]),
        template: "%USERPROFILE%",
    },
    // CSIDL_DESKTOPDIRECTORY, FOLDERID_Desktop {B4BFCC3A-DB2C-424C-B029-7FE99A87C641}
    Folder {
        csidl: 0x10,
        id: g(0xB4BF_CC3A, 0xDB2C, 0x424C, [0xB0, 0x29, 0x7F, 0xE9, 0x9A, 0x87, 0xC6, 0x41]),
        template: "%USERPROFILE%\\Desktop",
    },
    // CSIDL_APPDATA, FOLDERID_RoamingAppData {3EB685DB-65F9-4CF6-A03A-E3EF65729F3D}
    Folder {
        csidl: 0x1A,
        id: g(0x3EB6_85DB, 0x65F9, 0x4CF6, [0xA0, 0x3A, 0xE3, 0xEF, 0x65, 0x72, 0x9F, 0x3D]),
        template: "%APPDATA%",
    },
    // CSIDL_LOCAL_APPDATA, FOLDERID_LocalAppData {F1B32785-6FBA-4FCF-9D55-7B8E7F157091}
    Folder {
        csidl: 0x1C,
        id: g(0xF1B3_2785, 0x6FBA, 0x4FCF, [0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91]),
        template: "%LOCALAPPDATA%",
    },
    // CSIDL_PROGRAMS, FOLDERID_Programs {A77F5D77-2E2B-44C3-A6A2-ABA601054A51}
    Folder {
        csidl: 0x02,
        id: g(0xA77F_5D77, 0x2E2B, 0x44C3, [0xA6, 0xA2, 0xAB, 0xA6, 0x01, 0x05, 0x4A, 0x51]),
        template: "%APPDATA%\\Microsoft\\Windows\\Start Menu\\Programs",
    },
];

/// `CSIDL_DRIVES` and `FOLDERID_ComputerFolder` {0AC0837C-BBF8-452A-850D-79D08E667CA7}: the
/// folder a drive path is under when no deeper one holds it.
const COMPUTER: Folder = Folder {
    csidl: 0x11,
    id: g(0x0AC0_837C, 0xBBF8, 0x452A, [0x85, 0x0D, 0x79, 0xD0, 0x8E, 0x66, 0x7C, 0xA7]),
    template: "",
};

/// The deepest folder that holds `path` (below it, not the folder itself) and the number of
/// path components the folder has.
fn folder_of(path: &[u16], sys: &dyn System) -> (&'static Folder, usize) {
    let parts: Vec<&[u16]> = path[3..].split(|&c| c == u16::from(b'\\')).filter(|p| !p.is_empty()).collect();
    let mut best: (&Folder, usize) = (&COMPUTER, 0);
    for f in FOLDERS {
        let p = text::expand_env(&text::wide(f.template), |n| sys.env(n));
        if p.contains(&u16::from(b'%'))
            || !text::is_drive_path(&p)
            || text::drive_letter(&p) != text::drive_letter(path)
        {
            continue;
        }
        let fparts: Vec<&[u16]> = p[3..].split(|&c| c == u16::from(b'\\')).filter(|p| !p.is_empty()).collect();
        let below = fparts.len() < parts.len() && fparts.iter().zip(&parts).all(|(a, b)| text::eq_nocase(a, b));
        if below && fparts.len() > best.1 {
            best = (f, fparts.len());
        }
    }
    best
}

fn put_u32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

/// An `EnvironmentVariableDataBlock` or `IconEnvironmentDataBlock` (788 bytes): `TargetAnsi`
/// (260 bytes) and `TargetUnicode` (260 characters), NUL-terminated and cut to fit.
fn environment_block(v: &mut Vec<u8>, signature: u32, t: &[u16]) {
    put_u32(v, 0x314);
    put_u32(v, signature);
    let start = v.len();
    let ansi = text::to_ansi(&t[..t.len().min(259)]);
    v.extend_from_slice(&ansi);
    v.resize(start + 260, 0);
    for &c in &t[..t.len().min(259)] {
        v.extend_from_slice(&c.to_le_bytes());
    }
    v.resize(start + 260 + 520, 0);
}

/// A GUID made from text (FNV-1a over the units without case), for the tracker block's object id.
fn object_guid(path: &[u16]) -> Guid {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for &c in path {
        let c = if (u16::from(b'a')..=u16::from(b'z')).contains(&c) { c - 32 } else { c };
        for b in c.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01B3);
        }
    }
    let b = h.to_le_bytes();
    Guid { data1: (h >> 32) as u32, data2: h as u16, data3: 0x4000 | ((h >> 16) as u16 & 0x0FFF), data4: b }
}

pub(crate) fn save(link: &Link, file: Option<&[u16]>, sys: &dyn System) -> Vec<u8> {
    let path = link.path.as_deref().filter(|p| !p.is_empty());
    let target = path.and_then(|p| sys.entry(p)).map(|(_, info)| info);
    let drive_path = path.filter(|p| text::is_drive_path(p));
    let drive = drive_path.and_then(text::drive_letter);
    let drive_exists = drive.is_some_and(|d| sys.drive_exists(d));
    let unc = path.filter(|p| text::is_unc(p));
    let mut f = flags::IS_UNICODE;

    // The target's ID list.
    let mut list: Option<(Vec<u8>, Vec<usize>)> = None;
    if let (Some(p), Some(d)) = (drive_path, drive) {
        let names: Vec<&[u16]> = p[3..].split(|&c| c == u16::from(b'\\')).filter(|n| !n.is_empty()).collect();
        let mut prefix: Text = p[..3].to_vec();
        let mut parts = Vec::new();
        for (i, name) in names.iter().enumerate() {
            if i > 0 {
                prefix.push(u16::from(b'\\'));
            }
            prefix.extend_from_slice(name);
            let last = i + 1 == names.len();
            let info = if last { target } else { sys.entry(&prefix).map(|(_, info)| info) };
            let folder = if last { target.is_some_and(|t| t.is_dir()) } else { true };
            parts.push(Component { name, folder, info: info.unwrap_or_default() });
        }
        list = Some(idlist::build(d, &parts));
        f |= flags::HAS_LINK_TARGET_ID_LIST;
    }

    // LinkInfo: a target that exists on a local disk.
    let mut link_info = Vec::new();
    if let (Some(p), Some(d), Some(_)) = (drive_path, drive, target) {
        let base = text::to_ansi(p);
        let total = 28 + 17 + base.len() + 1 + 1;
        put_u32(&mut link_info, total as u32);
        put_u32(&mut link_info, 0x1C);
        put_u32(&mut link_info, 1); // VolumeIDAndLocalBasePath
        put_u32(&mut link_info, 0x1C);
        put_u32(&mut link_info, 0x1C + 17);
        put_u32(&mut link_info, 0);
        put_u32(&mut link_info, (0x1C + 17 + base.len() + 1) as u32);
        put_u32(&mut link_info, 17);
        put_u32(&mut link_info, 3); // DRIVE_FIXED
        put_u32(&mut link_info, sys.volume_serial(d));
        put_u32(&mut link_info, 0x10);
        link_info.push(0);
        link_info.extend_from_slice(&base);
        link_info.extend_from_slice(&[0, 0]);
        f |= flags::HAS_LINK_INFO;
    }

    // The strings.
    let relative = match (drive_path.filter(|_| drive_exists), link.relative_source.as_deref().or(file)) {
        (Some(p), Some(base)) => text::relative_path(p, base),
        (Some(_), None) => None,
        (None, _) => None,
    }
    .or_else(|| link.relative_path.clone().filter(|_| list.is_none()));
    let strings: [(u32, Option<&[u16]>); 5] = [
        (flags::HAS_NAME, link.description.as_deref()),
        (flags::HAS_RELATIVE_PATH, relative.as_deref()),
        (flags::HAS_WORKING_DIR, link.working_dir.as_deref()),
        (flags::HAS_ARGUMENTS, link.arguments.as_deref()),
        (flags::HAS_ICON_LOCATION, link.icon_path.as_deref()),
    ];
    for (bit, s) in &strings {
        if s.is_some_and(|s| !s.is_empty()) {
            f |= bit;
        }
    }
    let icon_block = link
        .icon_path
        .as_deref()
        .filter(|i| (text::is_drive_path(i) || text::is_unc(i)) && !i.contains(&u16::from(b'%')));
    if icon_block.is_some() {
        f |= flags::HAS_EXP_ICON;
    }
    let env_block = link.env_path.as_deref().or(unc);
    if env_block.is_some() {
        f |= flags::HAS_EXP_STRING;
    }

    // The header.
    let info = target.unwrap_or_default();
    let mut v = Vec::with_capacity(1024);
    put_u32(&mut v, HEADER_SIZE as u32);
    v.extend_from_slice(&guid_bytes(&CLSID_SHELL_LINK));
    put_u32(&mut v, f);
    put_u32(&mut v, info.attributes);
    for t in [info.creation, info.access, info.write] {
        v.extend_from_slice(&t.to_le_bytes());
    }
    put_u32(&mut v, info.size as u32);
    put_u32(&mut v, link.icon_index as u32);
    put_u32(&mut v, file_show_command(link.show_command) as u32);
    v.extend_from_slice(&link.hotkey.to_le_bytes());
    v.extend_from_slice(&[0; 10]);
    debug_assert_eq!(v.len(), HEADER_SIZE);

    if let Some((items, _)) = &list {
        v.extend_from_slice(&(items.len() as u16).to_le_bytes());
        v.extend_from_slice(items);
    }
    v.extend_from_slice(&link_info);
    for (_, s) in strings {
        if let Some(s) = s.filter(|s| !s.is_empty()) {
            let s = &s[..s.len().min(0xFFFF)];
            v.extend_from_slice(&(s.len() as u16).to_le_bytes());
            for c in s {
                v.extend_from_slice(&c.to_le_bytes());
            }
        }
    }

    // The extra data blocks.
    if let Some(i) = icon_block {
        environment_block(&mut v, block::ICON_ENVIRONMENT, i);
    }
    if let Some(e) = env_block {
        environment_block(&mut v, block::ENVIRONMENT, e);
    }
    if let Some(u) = unc {
        // VistaAndAboveIDListDataBlock: Poly's one-item list of the UNC path.
        let size = 8 + 2 + 2 * u.len() + 2;
        put_u32(&mut v, size as u32);
        put_u32(&mut v, block::VISTA_ID_LIST);
        v.extend_from_slice(&((2 + 2 * u.len()) as u16).to_le_bytes());
        for c in u {
            v.extend_from_slice(&c.to_le_bytes());
        }
        v.extend_from_slice(&[0, 0]);
    }
    let known_drive = list.as_ref().filter(|_| drive_exists);
    if let (Some((_, sizes)), Some(p)) = (known_drive, drive_path) {
        let (folder, depth) = folder_of(p, sys);
        let offset: usize = sizes.iter().take(if depth == 0 { 1 } else { 2 + depth }).sum();
        put_u32(&mut v, 0x10);
        put_u32(&mut v, block::SPECIAL_FOLDER);
        put_u32(&mut v, folder.csidl);
        put_u32(&mut v, offset as u32);
        put_u32(&mut v, 0x1C);
        put_u32(&mut v, block::KNOWN_FOLDER);
        v.extend_from_slice(&guid_bytes(&folder.id));
        put_u32(&mut v, offset as u32);
    }
    if let (Some(p), Some(d), Some(_)) = (drive_path, drive, target) {
        // TrackerDataBlock (MS-SHLLINK 2.5.10): the machine and the volume and object ids.
        put_u32(&mut v, 0x60);
        put_u32(&mut v, block::TRACKER);
        put_u32(&mut v, 0x58);
        put_u32(&mut v, 0);
        let mut machine = sys.machine_name();
        machine.truncate(15);
        machine.resize(16, 0);
        v.extend_from_slice(&machine);
        let ids = [volume_guid(sys.volume_serial(d)), object_guid(p)];
        for _ in 0..2 {
            for id in &ids {
                v.extend_from_slice(&guid_bytes(id));
            }
        }
    }
    let stored = link.properties.entries().iter().any(|(_, value)| *value != crate::PropValue::Empty);
    if known_drive.is_some() || stored {
        let sid = sys.user_sid();
        let store = link.properties.serialize(Some(&sid));
        put_u32(&mut v, (8 + store.len()) as u32);
        put_u32(&mut v, block::PROPERTY_STORE);
        v.extend_from_slice(&store);
    }
    put_u32(&mut v, 0);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_ids_are_the_known_folder_tables() {
        use poly_kernelbase::knownfolder::KNOWN_FOLDERS;
        for (name, f) in [
            ("Windows", &FOLDERS[0]),
            ("System", &FOLDERS[1]),
            ("ProgramFiles", &FOLDERS[2]),
            ("ProgramFilesX86", &FOLDERS[3]),
            ("ProgramData", &FOLDERS[4]),
            ("CommonPrograms", &FOLDERS[5]),
            ("PublicDesktop", &FOLDERS[6]),
            ("Profile", &FOLDERS[7]),
            ("Desktop", &FOLDERS[8]),
            ("RoamingAppData", &FOLDERS[9]),
            ("LocalAppData", &FOLDERS[10]),
            ("Programs", &FOLDERS[11]),
            ("ComputerFolder", &COMPUTER),
        ] {
            let entry = KNOWN_FOLDERS.iter().find(|k| k.0 == name).expect(name);
            assert_eq!(entry.1, guid_bytes(&f.id), "{name}");
        }
    }

    #[test]
    fn object_ids_do_not_depend_on_case() {
        assert_eq!(object_guid(&text::wide("C:\\A\\b.exe")), object_guid(&text::wide("c:\\a\\B.EXE")));
    }
}
