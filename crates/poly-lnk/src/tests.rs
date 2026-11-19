//! The sheets' rules on a made-up system: `C:` with Windows, System32, kernel32.dll and the
//! Public desktop; no other drive.

use super::*;
use crate::props::{PKEY_APP_USER_MODEL_ID, PKEY_COMMENT, PKEY_TOAST_ACTIVATOR_CLSID, PKEY_VOLUME_ID};
use crate::read::{block, flags};
use crate::text::{eq_nocase, wide};
use alloc::string::String;

struct Fake;

const DIRS: &[&str] = &[
    "C:\\",
    "C:\\Windows",
    "C:\\Windows\\System32",
    "C:\\Users",
    "C:\\Users\\Public",
    "C:\\Users\\Public\\Desktop",
    "C:\\Program Files",
    "C:\\ProgramData",
];

impl System for Fake {
    fn env(&self, name: &[u16]) -> Option<Text> {
        let value = match String::from_utf16_lossy(name).to_ascii_uppercase().as_str() {
            "SYSTEMROOT" => "C:\\Windows",
            "PROGRAMFILES" => "C:\\Program Files",
            "PROGRAMDATA" => "C:\\ProgramData",
            "PUBLIC" => "C:\\Users\\Public",
            "USERPROFILE" => "C:\\Users\\poly",
            _ => return None,
        };
        Some(wide(value))
    }
    fn entry(&self, path: &[u16]) -> Option<(Text, FileInfo)> {
        let name = |p: &str| wide(p.rsplit('\\').next().unwrap_or(p));
        for d in DIRS {
            if eq_nocase(path, &wide(d)) {
                return Some((name(d), FileInfo { attributes: 0x10, ..FileInfo::default() }));
            }
        }
        let k = "C:\\Windows\\System32\\kernel32.dll";
        eq_nocase(path, &wide(k))
            .then(|| (name(k), FileInfo { attributes: 0x20, size: 0x8_0000, write: 1, ..FileInfo::default() }))
    }
    fn drive_exists(&self, letter: u16) -> bool {
        letter == u16::from(b'C')
    }
    fn volume_serial(&self, _: u16) -> u32 {
        0x1234_5678
    }
    fn machine_name(&self) -> Vec<u8> {
        b"POLY".to_vec()
    }
    fn user_sid(&self) -> Text {
        wide("S-1-5-21-520193847-884726105-1936024571-1002")
    }
}

fn path_of(link: &Link, raw: bool) -> String {
    String::from_utf16_lossy(link.reported_path(raw).unwrap_or(&[]))
}

fn set(text: &str) -> (SetPath, Link) {
    let mut l = Link::new();
    let r = l.set_path(&wide(text), &Fake);
    (r, l)
}

#[test]
fn set_path_follows_the_sheet() {
    let (r, l) = set("c:\\windows\\system32\\KERNEL32.DLL");
    assert_eq!(r, SetPath::Done);
    assert_eq!(path_of(&l, false), "C:\\Windows\\System32\\kernel32.dll");
    assert_eq!(l.find.attributes, 0x20);
    assert_eq!(l.properties.key_at(0), Some(PKEY_VOLUME_ID));

    for odd in [
        "C:\\Windows\\System32\\..\\System32\\kernel32.dll",
        "C:/Windows/System32/kernel32.dll",
        "\"C:\\Windows\\System32\\kernel32.dll\"",
    ] {
        let (r, l) = set(odd);
        assert_eq!(r, SetPath::Done);
        assert_eq!(path_of(&l, false), "C:\\Windows\\System32\\kernel32.dll", "{odd}");
        assert_eq!(l.find, FileInfo::default(), "{odd}");
    }

    let (r, l) = set("%SystemRoot%\\System32\\kernel32.dll");
    assert_eq!(r, SetPath::Done);
    assert_eq!(path_of(&l, false), "C:\\Windows\\System32\\kernel32.dll");
    assert_eq!(path_of(&l, true), "%SystemRoot%\\System32\\kernel32.dll");
    assert_eq!(l.find.attributes, 0x20);

    let (r, l) = set("\"C:\\Windows\\System32\\kernel32.dll\" /c exit");
    assert_eq!(r, SetPath::InvalidArg);
    assert_eq!(l.path, None);

    let (_, l) = set("C:\\Windows\\System32\\kernel32.dll /c exit");
    assert_eq!(path_of(&l, false), "C:\\Windows\\System32\\kernel32.dll \\c exit");
    assert_eq!(text::file_name(l.path.as_deref().unwrap()), wide("c exit"));

    let (_, l) = set("C:\\Windows\\System32");
    assert_eq!(l.find.attributes, 0x10);
    let (_, l) = set("c:\\windows\\PolySpecB4-missing.exe");
    assert_eq!(path_of(&l, false), "c:\\windows\\PolySpecB4-missing.exe");
    let (_, l) = set("\\\\PolySpecB4Server\\share\\file.txt");
    assert_eq!(path_of(&l, false), "\\\\PolySpecB4Server\\share\\file.txt");
    let (r, l) = set("");
    assert_eq!((r, l.path), (SetPath::Done, None));

    let long = alloc::format!("C:\\{}", "a".repeat(256));
    assert_eq!(set(&long).0, SetPath::Done);
    let long = alloc::format!("C:\\{}", "a".repeat(257));
    let (r, l) = set(&long);
    assert_eq!((r, l.path), (SetPath::TooLong, None));
}

#[test]
fn resolve_follows_the_sheet() {
    let mut l = Link::new();
    assert_eq!(l.resolve(&Fake), hr::S_OK);
    l.set_path(&wide("C:\\Windows\\System32\\kernel32.dll"), &Fake);
    assert_eq!(l.resolve(&Fake), hr::S_OK);
    l.set_path(&wide("Q:\\PolySpecB4\\app.exe"), &Fake);
    assert_eq!(l.resolve(&Fake), hr::S_FALSE);
    assert_eq!(path_of(&l, false), "Q:\\PolySpecB4\\app.exe");
}

#[test]
fn buffer_rules() {
    let p = Some(32);
    assert_eq!(text::get_path(p, 0), (hr::E_INVALIDARG, None));
    assert_eq!(text::get_path(p, -1), (hr::E_INVALIDARG, Some(0)));
    assert_eq!(text::get_path(p, 1), (hr::E_INSUFFICIENT_BUFFER, Some(0)));
    assert_eq!(text::get_path(p, 32), (hr::E_INSUFFICIENT_BUFFER, Some(31)));
    assert_eq!(text::get_path(p, 33), (hr::S_OK, Some(32)));
    assert_eq!(text::get_path(None, 0), (hr::S_FALSE, Some(0)));
    assert_eq!(text::truncated(19, 0), None);
    assert_eq!(text::truncated(19, 1), Some(0));
    assert_eq!(text::truncated(19, 5), Some(4));
    assert_eq!(text::truncated(19, 100), Some(19));
}

fn u32_at(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}

/// The extra block signatures of a file Save wrote.
fn blocks(d: &[u8]) -> Vec<u32> {
    // Walk from the end: blocks are found by scanning from the strings, so rebuild with the reader's
    // rules instead: start after the strings of the flags.
    let f = u32_at(d, 0x14);
    let mut at = 0x4C;
    if f & flags::HAS_LINK_TARGET_ID_LIST != 0 {
        at += 2 + usize::from(u16::from_le_bytes([d[at], d[at + 1]]));
    }
    if f & flags::HAS_LINK_INFO != 0 {
        at += u32_at(d, at) as usize;
    }
    for bit in 2..7 {
        if f & (1 << bit) != 0 {
            at += 2 + 2 * usize::from(u16::from_le_bytes([d[at], d[at + 1]]));
        }
    }
    let mut out = Vec::new();
    while u32_at(d, at) >= 8 {
        out.push(u32_at(d, at + 4));
        at += u32_at(d, at) as usize;
    }
    assert_eq!(at + 4, d.len());
    out
}

fn chrome_like(target: &str, dir: &str) -> Link {
    let mut l = Link::new();
    l.set_path(&wide(target), &Fake);
    l.arguments = Some(wide("--source-shortcut-location=desktop"));
    l.description = Some(wide("Access the Internet"));
    l.icon_path = Some(wide(target));
    l.icon_index = 0;
    l.working_dir = Some(wide(dir));
    l.set_property(&PKEY_APP_USER_MODEL_ID, PropValue::Text(wide("Chrome")));
    l
}

#[test]
fn chromes_shortcut_is_written_as_on_windows() {
    let file = wide("C:\\Users\\Public\\Desktop\\PolySpecB4-Chrome.lnk");
    // A target that does not exist.
    let l = chrome_like(
        "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
        "C:\\Program Files\\Google\\Chrome\\Application",
    );
    let d = l.save(Some(&file), &Fake);
    assert_eq!(d.len(), 2061);
    assert_eq!(u32_at(&d, 0x14), 0x40FD);
    assert_eq!(
        blocks(&d),
        [block::ICON_ENVIRONMENT, block::SPECIAL_FOLDER, block::KNOWN_FOLDER, block::PROPERTY_STORE]
    );
    let back = Link::load(&d, &Fake).unwrap();
    assert_eq!(path_of(&back, false), "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe");
    assert_eq!(back.arguments, l.arguments);
    assert_eq!(back.description, l.description);
    assert_eq!(back.working_dir, l.working_dir);
    assert_eq!(back.icon(), Some((&wide("C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe")[..], 0)));
    assert_eq!(back.app_user_model_id(), Some(&wide("Chrome")[..]));
    assert_eq!(back.relative_path, Some(wide("..\\..\\..\\Program Files\\Google\\Chrome\\Application\\chrome.exe")));

    // A target that exists: LinkInfo and the tracker block too.
    let l = chrome_like("C:\\Windows\\System32\\kernel32.dll", "C:\\Windows\\System32");
    let d = l.save(Some(&file), &Fake);
    assert_eq!(d.len(), 1975);
    assert_eq!(u32_at(&d, 0x14), 0x40FF);
    assert_eq!(u32_at(&d, 0x18), 0x20);
    assert_eq!(
        blocks(&d),
        [block::ICON_ENVIRONMENT, block::SPECIAL_FOLDER, block::KNOWN_FOLDER, block::TRACKER, block::PROPERTY_STORE]
    );
    let back = parse(&d).unwrap();
    assert_eq!(back.target(), Some(&wide("C:\\Windows\\System32\\kernel32.dll")[..]));
    assert_eq!(back.link_info_path, Some(wide("C:\\Windows\\System32\\kernel32.dll")));
}

#[test]
fn an_empty_link_is_80_bytes_and_a_description_118() {
    let d = Link::new().save(Some(&wide("C:\\x.lnk")), &Fake);
    assert_eq!(d.len(), 80);
    assert_eq!(u32_at(&d, 0x14), 0x80);
    assert_eq!(u32_at(&d, 0x3C), 1);
    let mut l = Link::new();
    l.description = Some(wide("a description here"));
    assert_eq!(l.save(None, &Fake).len(), 118);
}

#[test]
fn the_relative_path_from_beside_the_target() {
    let mut l = Link::new();
    l.set_path(&wide("C:\\Windows\\System32\\kernel32.dll"), &Fake);
    l.relative_source = Some(wide("C:\\Windows\\x.lnk"));
    let back = parse(&l.save(Some(&wide("C:\\Windows\\x.lnk")), &Fake)).unwrap();
    assert_eq!(back.relative_path, Some(wide(".\\System32\\kernel32.dll")));
    // A missing drive: an ID list, no relative path, no blocks.
    let mut l = Link::new();
    l.set_path(&wide("Q:\\PolySpecB4\\app.exe"), &Fake);
    let d = l.save(Some(&wide("C:\\Windows\\x.lnk")), &Fake);
    assert_eq!(u32_at(&d, 0x14), 0x81);
    assert_eq!(blocks(&d), [] as [u32; 0]);
}

#[test]
fn environment_paths_and_icons() {
    let mut l = Link::new();
    l.set_path(&wide("%SystemRoot%\\System32\\kernel32.dll"), &Fake);
    l.icon_path = Some(wide("%SystemRoot%\\System32\\shell32.dll"));
    l.icon_index = -5;
    let d = l.save(Some(&wide("C:\\Users\\Public\\Desktop\\x.lnk")), &Fake);
    let f = u32_at(&d, 0x14);
    assert!(f & flags::HAS_EXP_STRING != 0 && f & flags::HAS_EXP_ICON == 0);
    assert_eq!(blocks(&d)[0], block::ENVIRONMENT);
    let back = Link::load(&d, &Fake).unwrap();
    assert_eq!(path_of(&back, false), "C:\\Windows\\System32\\kernel32.dll");
    assert_eq!(path_of(&back, true), "%SystemRoot%\\System32\\kernel32.dll");
    assert_eq!(back.icon(), Some((&wide("%SystemRoot%\\System32\\shell32.dll")[..], -5)));

    // A UNC target: only the environment block names it.
    let mut l = Link::new();
    l.set_path(&wide("\\\\server\\share\\file.txt"), &Fake);
    let d = l.save(None, &Fake);
    assert_eq!(u32_at(&d, 0x14), 0x280);
    let back = parse(&d).unwrap();
    assert_eq!(back.path, None);
    assert_eq!(back.target(), Some(&wide("\\\\server\\share\\file.txt")[..]));
}

#[test]
fn headers_that_are_not_links_are_refused() {
    assert_eq!(parse(&[]), Ok(Link::new()));
    assert_eq!(parse(&[0; 0x4C]), Err(Error::NotALink));
    let good = Link::new().save(None, &Fake);
    assert_eq!(parse(&good[..0x4B]), Err(Error::NotALink));
    let mut bad = good.clone();
    bad[0] = 0x4D;
    assert_eq!(parse(&bad), Err(Error::NotALink));
    let mut bad = good.clone();
    bad[4] = 0;
    assert_eq!(parse(&bad), Err(Error::NotALink));
    // No terminal block, or bytes after it: fine.
    assert!(parse(&good[..good.len() - 4]).is_ok());
    let mut more = good.clone();
    more.extend_from_slice(&[1, 2, 3]);
    assert!(parse(&more).is_ok());
}

#[test]
fn show_commands_a_file_keeps() {
    for (set, kept) in [(1, 1), (3, 3), (7, 7), (2, 1), (0, 1), (9, 1), (-1, 1)] {
        let mut l = Link::new();
        l.show_command = set;
        assert_eq!(parse(&l.save(None, &Fake)).unwrap().show_command, kept);
    }
}

#[test]
fn an_icon_environment_block_wins_and_is_expanded_on_load() {
    let mut l = Link::new();
    l.icon_path = Some(wide("C:\\wrong\\icon.dll"));
    l.icon_index = 5;
    let mut d = l.save(None, &Fake);
    // Turn the block's text into one with a variable, as pf_built does.
    let at = d.windows(4).position(|w| w == 0xA000_0007u32.to_le_bytes()).unwrap() + 4;
    let t = wide("%SystemRoot%\\System32\\imageres.dll");
    d[at..at + 260].fill(0);
    d[at + 260..at + 780].fill(0);
    for (i, c) in t.iter().enumerate() {
        d[at + i] = *c as u8;
        d[at + 260 + 2 * i..at + 262 + 2 * i].copy_from_slice(&c.to_le_bytes());
    }
    let raw = parse(&d).unwrap();
    assert_eq!(raw.icon(), Some((&t[..], 5)));
    let loaded = Link::load(&d, &Fake).unwrap();
    assert_eq!(loaded.icon_path, Some(wide("C:\\Windows\\System32\\imageres.dll")));
}

#[test]
fn the_property_store_as_ipropertystore_shows_it() {
    let mut l = Link::new();
    assert_eq!(l.property_count(), 0);
    assert_eq!(l.property(&PKEY_APP_USER_MODEL_ID), None);
    l.description = Some(wide("Access the Internet"));
    assert_eq!(l.property_count(), 1);
    assert_eq!(l.property_key_at(0), Some(PKEY_COMMENT));
    assert_eq!(l.set_property(&PKEY_APP_USER_MODEL_ID, PropValue::Text(wide("Chrome"))), hr::S_OK);
    assert_eq!(l.property_count(), 2);
    assert_eq!(l.property_key_at(0), Some(PKEY_APP_USER_MODEL_ID));
    assert_eq!(l.set_property(&PKEY_APP_USER_MODEL_ID, PropValue::Empty), hr::S_OK);
    assert_eq!(l.property_count(), 2);
    assert_eq!(l.property(&PKEY_APP_USER_MODEL_ID), Some(PropValue::Empty));
    let toast = wide("{A6B5E3B5-0000-4A0B-8000-112233445566}");
    assert_eq!(l.set_property(&PKEY_TOAST_ACTIVATOR_CLSID, PropValue::Text(toast.clone())), hr::S_FALSE);
    assert_eq!(l.toast_activator_clsid(), Guid::parse(&toast));
    // The comment is not written into the file.
    let back = parse(&l.save(None, &Fake)).unwrap();
    // The empty AppUserModel.ID is left out; the user's SID is added.
    assert_eq!(back.properties.len(), 2);
    assert_eq!(back.properties.get(&PKEY_COMMENT), None);
    assert_eq!(back.toast_activator_clsid(), Guid::parse(&toast));
}
