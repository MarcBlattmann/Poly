use super::apps::{self, Entry, Files, Seen, Uninstall};
use super::program::{self, Image, Pick};
use super::testfiles::{dib4, dib32, fake_png, icon_group, pe, version_info};
use crate::format::{self, Format};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use poly_lnk::Link;
use std::collections::BTreeMap;

fn reader(bytes: &[u8]) -> impl FnMut(u64, &mut [u8]) -> usize + '_ {
    move |at, buf| {
        let at = at as usize;
        if at >= bytes.len() {
            return 0;
        }
        let n = buf.len().min(bytes.len() - at);
        buf[..n].copy_from_slice(&bytes[at..at + n]);
        n
    }
}

fn chrome_like() -> Vec<u8> {
    let version = version_info(
        [157, 0, 8103, 38],
        "040904b0",
        &[
            ("CompanyName", "Example Corp"),
            ("FileDescription", "Example Browser Installer"),
            ("FileVersion", "157.0.8103.38"),
            ("ProductName", "Example Browser"),
            ("ProductVersion", "157.0.8103.38"),
            ("OriginalFilename", "ExampleSetup.exe"),
        ],
    );
    let small = fake_png(1);
    let big = fake_png(2);
    let group = icon_group(&[(16, 32, small.len() as u32, 1), (0, 32, big.len() as u32, 2)]);
    // WIN_CERTIFICATE: dwLength, wRevision 0x200, wCertificateType 2, bytes.
    let mut cert = Vec::new();
    cert.extend_from_slice(&16u32.to_le_bytes());
    cert.extend_from_slice(&0x200u16.to_le_bytes());
    cert.extend_from_slice(&2u16.to_le_bytes());
    cert.extend_from_slice(&[0x30, 0, 0, 0, 0, 0, 0, 0]);
    pe(
        32,
        false,
        true,
        &[(16, 1, 0x409, &version), (3, 1, 0x409, &small), (3, 2, 0x409, &big), (14, 101, 0x409, &group)],
        &cert,
    )
}

#[test]
fn windows_programs_and_installer_packages_are_known_by_their_magic() {
    let exe = chrome_like();
    assert_eq!(format::detect(&exe[..format::HEAD]), Some(Format::WindowsProgram));
    // A DOS header whose PE header is past what was read still counts.
    let mut far = alloc::vec![0u8; 256];
    far[..2].copy_from_slice(b"MZ");
    far[0x3C..0x40].copy_from_slice(&0x400u32.to_le_bytes());
    assert_eq!(format::detect(&far), Some(Format::WindowsProgram));
    // ... one that points at something that is not a PE header does not.
    far[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    assert_eq!(format::detect(&far), None);
    assert_eq!(format::detect(&program::CFB_SIGNATURE), Some(Format::WindowsInstaller));
    assert_eq!(Format::WindowsProgram.not_yet(), None);
    assert_eq!(Format::WindowsInstaller.not_yet(), None);
    assert_eq!(Format::WindowsProgram.name(), "Windows program");
}

/// A compound file with a root entry of class `clsid` ([MS-CFB]: 512-byte
/// sectors, the directory in sector 1).
fn compound(clsid: [u8; 16]) -> Vec<u8> {
    let mut b = alloc::vec![0u8; 512 * 3];
    b[..8].copy_from_slice(&program::CFB_SIGNATURE);
    b[0x1E..0x20].copy_from_slice(&9u16.to_le_bytes());
    b[0x30..0x34].copy_from_slice(&1u32.to_le_bytes());
    let root = 512 * 2;
    for (i, u) in "Root Entry".encode_utf16().enumerate() {
        b[root + 2 * i..root + 2 * i + 2].copy_from_slice(&u.to_le_bytes());
    }
    b[root + 0x42] = 5;
    b[root + 0x50..root + 0x60].copy_from_slice(&clsid);
    b
}

#[test]
fn a_compound_file_is_an_msi_only_by_its_root_class() {
    let msi = compound(program::MSI_CLSID);
    assert!(program::is_msi(&mut reader(&msi)));
    // A Word document is a compound file too.
    let doc = compound([6, 9, 2, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    assert!(!program::is_msi(&mut reader(&doc)));
    assert!(!program::is_msi(&mut reader(&msi[..600])));
    assert!(!program::is_msi(&mut reader(b"MZ")));
}

#[test]
fn the_version_resource_names_the_program() {
    let exe = chrome_like();
    let p = program::read_program("/root/Downloads/ExampleSetup.exe", exe.len() as u64, &mut reader(&exe)).unwrap();
    assert_eq!(p.name, "Example Browser");
    assert_eq!(p.description, "Example Browser Installer");
    assert_eq!(p.company, "Example Corp");
    assert_eq!(p.version, "157.0.8103.38");
    assert_eq!((p.bits, p.gui, p.installer, p.signed), (32, true, true, true));
    assert_eq!(p.kind(), "Windows installer (32-bit)");
    // A signature without a signer Poly can read.
    assert_eq!(p.publisher, None);
    assert_eq!(p.publisher_text(), "Unknown publisher (signed)");
    // With one: the name, as the consent prompt says it (poly_x509::authenticode).
    let named = program::Program { publisher: Some("Google LLC".into()), ..p.clone() };
    assert_eq!(named.publisher_text(), "Google LLC (signature not verified)");
    // The largest image of the first group (0 is 256).
    assert_eq!(p.icon, Some(fake_png(2)));
}

#[test]
fn a_program_without_resources_is_named_by_its_file() {
    let exe = pe(64, false, false, &[], &[]);
    let p = program::read_program("tool.EXE", exe.len() as u64, &mut reader(&exe)).unwrap();
    assert_eq!((p.name.as_str(), p.version.as_str(), p.bits, p.gui), ("tool", "", 64, false));
    assert!(!p.installer && !p.signed && p.icon.is_none());
    assert_eq!(p.kind(), "Windows program (64-bit)");
    assert_eq!(p.publisher_text(), "Unknown publisher");
    // A file name that says "setup" makes it an installer.
    let p = program::read_program("/x/7z2409-x64 Setup.exe", 1, &mut reader(&exe)).unwrap();
    assert!(p.installer);
    // A DLL is no program; random bytes no image.
    let dll = pe(64, true, true, &[], &[]);
    assert_eq!(program::read_program("a.dll", 1, &mut reader(&dll)), Err(crate::Error::Library));
    assert_eq!(program::read_program("a.exe", 1, &mut reader(b"MZ but nothing")), Err(crate::Error::NotThisFormat));
}

#[test]
fn version_strings_fall_back_and_comma_versions_are_dotted() {
    let v = version_info([1, 2, 3, 4], "041104b0", &[("FileDescription", "Tool"), ("FileVersion", "1, 2, 3, 4")]);
    let exe = pe(64, false, true, &[(16, 1, 0x411, &v)], &[]);
    let p = program::read_program("t.exe", 9, &mut reader(&exe)).unwrap();
    assert_eq!((p.name.as_str(), p.version.as_str()), ("Tool", "1.2.3.4"));
    // Only the fixed version.
    let v = version_info([5, 6, 7, 8], "040904b0", &[]);
    let exe = pe(64, false, true, &[(16, 1, 0x409, &v)], &[]);
    let p = program::read_program("t.exe", 9, &mut reader(&exe)).unwrap();
    assert_eq!(p.version, "5.6.7.8");
    let info = program::parse_version(&v);
    assert_eq!((info.file_version, info.product_version), (Some([5, 6, 7, 8]), Some([5, 6, 7, 8])));
    // A broken block reads as nothing, without a panic.
    assert_eq!(program::parse_version(&v[..10]).strings, Vec::<(String, String)>::new());
    assert_eq!(program::parse_version(&[0xff; 64]), program::VersionInfo::default());
}

#[test]
fn icons_are_picked_by_index_or_id_and_dibs_become_pngs() {
    let blue = dib32(32, [0xff, 0, 0, 0xff]);
    let mut palette = [[0u8; 4]; 16];
    palette[3] = [0, 0x80, 0xff, 0];
    let orange = dib4(16, &palette, 3);
    let g1 = icon_group(&[(32, 32, blue.len() as u32, 7)]);
    let g2 = icon_group(&[(16, 4, orange.len() as u32, 8)]);
    let exe =
        pe(64, false, true, &[(3, 7, 0x409, &blue), (3, 8, 0x409, &orange), (14, 1, 0, &g1), (14, 50, 0, &g2)], &[]);
    let mut r = reader(&exe);
    let mut image = Image::open(&mut r).unwrap();
    assert_eq!(image.count(14), 2);
    assert!(image.resource(16, Pick::First).is_none());
    // Index 0: the first group, a 32-bit DIB made a PNG.
    let png = image.icon(0).unwrap();
    let decoded = poly_png::decode(&png).unwrap();
    assert_eq!((decoded.width, decoded.height), (32, 32));
    // poly-png gives 0xAARRGGBB.
    assert_eq!(decoded.pixels[0], 0xff00_00ff);
    // Index -50: the group with id 50; index 1: the second group. The left
    // half is transparent by the mask, the right half the palette's orange.
    for which in [-50, 1] {
        let png = image.icon(which).unwrap();
        let d = poly_png::decode(&png).unwrap();
        assert_eq!((d.width, d.height), (16, 16));
        assert_eq!(d.pixels[0] >> 24, 0, "left half transparent");
        assert_eq!(d.pixels[8], 0xffff_8000);
    }
    // An index past the groups falls back to the first.
    assert!(image.icon(9).is_some());
}

#[test]
fn a_dib_that_is_not_one_is_refused() {
    assert_eq!(program::dib_to_png(&[0; 10]), None);
    let mut d = dib32(4, [1, 2, 3, 4]);
    d[16] = 3; // BI_BITFIELDS: not read
    assert_eq!(program::dib_to_png(&d), None);
    let mut d = dib32(4, [1, 2, 3, 4]);
    d.truncate(50);
    assert_eq!(program::dib_to_png(&d), None);
}

// ----- shell links --------------------------------------------------------------------------------

/// Fixtures written by hand from [MS-SHLLINK] with a separate script
/// (`tests/fixtures/`): a link with a Unicode `LinkInfo`, every string and a
/// target ID list; an ANSI link (no `IsUnicode`, a `LinkInfo` header of
/// 0x1C bytes) with a `CommonPathSuffix`; one with only an environment
/// block and a relative path. They are read by `poly-lnk`, Poly's one
/// reader of shell links (ADR 0227), as the installer reads them.
const FIXTURE_UNICODE: &[u8] = include_bytes!("../../tests/fixtures/example-browser.lnk");
const FIXTURE_ANSI: &[u8] = include_bytes!("../../tests/fixtures/tool-ansi.lnk");
const FIXTURE_ENV: &[u8] = include_bytes!("../../tests/fixtures/editor-env.lnk");

fn text(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[test]
fn the_fixture_links_read_as_written() {
    let l = poly_lnk::parse(FIXTURE_UNICODE).unwrap();
    assert_eq!(l.target().map(text).as_deref(), Some("C:\\Program Files\\Example\\Browser\\Application\\browser.exe"));
    assert_eq!(text(l.arguments()), "--profile-directory=\"Default User\" --flag");
    assert_eq!(text(l.working_dir()), "C:\\Program Files\\Example\\Browser\\Application");
    assert_eq!(text(l.description()), "Access the Internet");
    let (icon, index) = l.icon().unwrap();
    assert_eq!((text(icon).as_str(), index), ("C:\\Program Files\\Example\\Browser\\Application\\browser.exe", 0));
    assert_eq!(l.show_command(), 1);

    let l = poly_lnk::parse(FIXTURE_ANSI).unwrap();
    assert_eq!(l.target().map(text).as_deref(), Some("C:\\Tools\\Caf\u{e9}\\tool.exe"));
    assert_eq!(text(l.arguments()), "/quiet");
    assert_eq!(l.icon_index, -101);
    assert_eq!(l.show_command(), 3);

    let l = poly_lnk::parse(FIXTURE_ENV).unwrap();
    assert_eq!(l.env_path.as_deref().map(text).as_deref(), Some("%ProgramFiles%\\Editor\\editor.exe"));
    assert_eq!(
        l.relative_path.as_deref().map(text).as_deref(),
        Some("..\\..\\..\\..\\..\\Program Files\\Editor\\editor.exe")
    );
    assert!(l.arguments().is_empty());
}

#[test]
fn what_is_not_a_link_makes_no_entry() {
    let d = disk();
    let why = apps::entry_from_file("/nt/c/Users/Public/Desktop/x.lnk", b"not a link at all", &d).unwrap_err();
    assert!(why.contains("NotALink"), "{why}");
    let mut wrong = FIXTURE_UNICODE.to_vec();
    wrong[4] ^= 1;
    assert!(apps::entry_from_file("/x.lnk", &wrong, &d).is_err());
    // A cut link makes no entry, or one whose program is there; never a panic.
    for cut in 0..FIXTURE_ENV.len() {
        let _ = apps::entry_from_file("/nt/c/Users/Public/Desktop/Editor.lnk", &FIXTURE_ENV[..cut], &d);
    }
    assert!(apps::is_link_name("Google Chrome.LNK") && !apps::is_link_name(".lnk") && !apps::is_link_name("a.exe"));
}

#[test]
fn a_relative_path_is_taken_from_the_links_folder() {
    let l = Link { relative_path: Some(wide("..\\App\\app.exe")), ..Link::default() };
    assert_eq!(apps::program(&l, "C:\\Users\\Public\\Desktop").as_deref(), Some("C:\\Users\\Public\\App\\app.exe"));
    assert_eq!(apps::program(&Link::default(), "C:\\"), None);
}

// ----- launcher entries ------------------------------------------------------------------------------

/// A made-up disk: directories and files.
#[derive(Default)]
struct Disk {
    dirs: BTreeMap<String, Vec<String>>,
    files: Vec<String>,
}

impl Disk {
    fn with(paths: &[&str]) -> Disk {
        let mut d = Disk::default();
        for p in paths {
            d.files.push(p.to_string());
            let mut at = String::new();
            for part in p.trim_start_matches('/').split('/') {
                let dir = if at.is_empty() { "/".to_string() } else { at.clone() };
                let names = d.dirs.entry(dir).or_default();
                if !names.iter().any(|n| n == part) {
                    names.push(part.to_string());
                }
                at = alloc::format!("{at}/{part}");
            }
        }
        d
    }
}

impl Files for Disk {
    fn list(&self, dir: &str) -> Vec<String> {
        self.dirs.get(dir).cloned().unwrap_or_default()
    }
    fn is_dir(&self, path: &str) -> bool {
        self.dirs.contains_key(path)
    }
    fn exists(&self, path: &str) -> bool {
        self.is_dir(path) || self.files.iter().any(|f| f == path)
    }
}

fn disk() -> Disk {
    Disk::with(&[
        "/nt/c/Program Files/Example/Browser/Application/browser.exe",
        "/nt/c/Program Files/Example/Browser/Application/157.0/setup.exe",
        "/nt/c/Program Files/Editor/editor.exe",
        "/nt/c/Tools/Caf\u{e9}/tool.exe",
        "/nt/c/ProgramData/Microsoft/Windows/Start Menu/Programs/Example Browser.lnk",
        "/nt/c/Users/Public/Desktop/Example Browser.lnk",
    ])
}

#[test]
fn windows_paths_map_to_polys_without_case() {
    let d = disk();
    assert_eq!(
        apps::vfs_path("c:\\PROGRAM FILES\\example\\browser\\application\\BROWSER.EXE", &d).as_deref(),
        Some("/nt/c/Program Files/Example/Browser/Application/browser.exe")
    );
    assert_eq!(
        apps::vfs_path("\"C:\\Tools\\..\\Tools\\Caf\u{c9}\\tool.exe\"", &d).as_deref(),
        Some("/nt/c/Tools/Caf\u{e9}/tool.exe")
    );
    assert_eq!(apps::vfs_path("D:\\x.exe", &d), None);
    assert_eq!(apps::vfs_path("C:\\Missing\\x.exe", &d), None);
    assert_eq!(apps::vfs_path("C:\\", &d).as_deref(), Some("/nt/c"));
    assert_eq!(apps::dos_path("/nt/c/Users/Public/Desktop").as_deref(), Some("C:\\Users\\Public\\Desktop"));
    assert_eq!(apps::dos_path("/nt/c").as_deref(), Some("C:\\"));
    assert_eq!(apps::dos_path("/nt/cx"), None);
}

#[test]
fn environment_variables_expand_to_the_nt_layout() {
    assert_eq!(apps::expand("%ProgramFiles%\\A"), "C:\\Program Files\\A");
    assert_eq!(apps::expand("%PROGRAMFILES(X86)%\\B"), "C:\\Program Files (x86)\\B");
    assert_eq!(apps::expand("%LocalAppData%\\C;%SystemRoot%"), "C:\\Users\\poly\\AppData\\Local\\C;C:\\Windows");
    assert_eq!(apps::expand("100% %unknown% 50%"), "100% %unknown% 50%");
}

#[test]
fn command_lines_split_as_windows_parses_them() {
    let s = |t: &str| apps::split_command_line(t);
    assert_eq!(s("a b\tc"), ["a", "b", "c"]);
    assert_eq!(s("--profile-directory=\"Default User\" --flag"), ["--profile-directory=Default User", "--flag"]);
    assert_eq!(s(r#""a\"b" c\\d "e\\" f"#), [r#"a"b"#, r#"c\\d"#, r#"e\"#, "f"]);
    assert_eq!(s(r#""x""y" """#), [r#"x"y"#, ""]);
    assert!(s("   ").is_empty());
}

#[test]
fn exec_arguments_are_quoted_as_the_desktop_entry_specification_says() {
    assert_eq!(apps::exec_arg("/nt/c/Program Files/x.exe"), "\"/nt/c/Program Files/x.exe\"");
    assert_eq!(apps::exec_arg("--a=50%"), "--a=50%%");
    assert_eq!(apps::exec_arg("a\"b$c`d\\e"), "\"a\\\"b\\$c\\`d\\\\e\"");
    assert_eq!(apps::exec_arg(""), "\"\"");
    // What the launcher reads back is the argument.
    let text = alloc::format!(
        "[Desktop Entry]\nType=Application\nName=X\nExec={} {}\n",
        apps::exec_arg("/nt/c/Program Files/x.exe"),
        apps::exec_arg("--p=\"A B\" 5%")
    );
    let app = poly_shell_parse(&text);
    assert_eq!(app, ["/nt/c/Program Files/x.exe", "--p=\"A B\" 5%"]);
}

/// The launcher's reading of `Exec` (Desktop Entry Specification): quoted
/// arguments with `\` escapes, `%%` for `%`.
fn poly_shell_parse(text: &str) -> Vec<String> {
    let exec = text.lines().find_map(|l| l.strip_prefix("Exec=")).unwrap();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = exec.chars().peekable();
    let mut have = false;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                have = true;
            }
            '\\' if quoted => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            '%' if chars.peek() == Some(&'%') => {
                chars.next();
                cur.push('%');
            }
            ' ' if !quoted => {
                if have || !cur.is_empty() {
                    out.push(core::mem::take(&mut cur));
                    have = false;
                }
            }
            c => cur.push(c),
        }
    }
    if have || !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn browser_entry(d: &Disk) -> Entry {
    let link = poly_lnk::parse(FIXTURE_UNICODE).unwrap();
    apps::entry_for("/nt/c/ProgramData/Microsoft/Windows/Start Menu/Programs/Example Browser.lnk", &link, d).unwrap()
}

#[test]
fn a_shortcut_becomes_a_launcher_entry() {
    let d = disk();
    let e = browser_entry(&d);
    assert_eq!(e.id, "poly-windows-example-browser.desktop");
    assert_eq!(e.name, "Example Browser");
    assert_eq!(e.comment, "Access the Internet");
    assert_eq!(
        e.argv,
        ["/nt/c/Program Files/Example/Browser/Application/browser.exe", "--profile-directory=Default User", "--flag"]
    );
    assert_eq!(e.working_dir.as_deref(), Some("/nt/c/Program Files/Example/Browser/Application"));
    assert_eq!(e.icon_source, ("/nt/c/Program Files/Example/Browser/Application/browser.exe".to_string(), 0));
    let text = apps::desktop_file(&e, Some("/root/.local/share/poly/windows/poly-windows-example-browser.png"), None);
    assert_eq!(
        text,
        "[Desktop Entry]\nType=Application\nName=Example Browser\nComment=Access the Internet\n\
         Exec=\"/nt/c/Program Files/Example/Browser/Application/browser.exe\" \"--profile-directory=Default User\" --flag\n\
         Path=/nt/c/Program Files/Example/Browser/Application\n\
         Icon=/root/.local/share/poly/windows/poly-windows-example-browser.png\nKeywords=windows;\n\
         X-Poly-Windows-Link=/nt/c/ProgramData/Microsoft/Windows/Start Menu/Programs/Example Browser.lnk\n\
         X-Poly-Windows-Target=C:\\Program Files\\Example\\Browser\\Application\\browser.exe\n"
    );
    assert_eq!(
        poly_shell_parse(&text),
        ["/nt/c/Program Files/Example/Browser/Application/browser.exe", "--profile-directory=Default User", "--flag"]
    );
    assert_eq!(
        apps::icon_path("/root/.local/share/", &e.id),
        "/root/.local/share/poly/windows/poly-windows-example-browser.png"
    );
    assert!(
        apps::is_windows_id(&e.id)
            && !apps::is_windows_id("poly-windows-.desktop")
            && !apps::is_windows_id("x.desktop")
    );
}

#[test]
fn an_environment_shortcut_and_one_with_a_missing_target() {
    let d = disk();
    let link = poly_lnk::parse(FIXTURE_ENV).unwrap();
    let e = apps::entry_for("/nt/c/Users/Public/Desktop/Editor.lnk", &link, &d).unwrap();
    assert_eq!(e.argv, ["/nt/c/Program Files/Editor/editor.exe"]);
    assert_eq!(e.target, "C:\\Program Files\\Editor\\editor.exe");
    assert_eq!(e.working_dir, None);
    // An ANSI link whose icon is a resource id.
    let link = poly_lnk::parse(FIXTURE_ANSI).unwrap();
    let e = apps::entry_for("/nt/c/Users/Public/Desktop/Tool.lnk", &link, &d).unwrap();
    assert_eq!(e.argv, ["/nt/c/Tools/Caf\u{e9}/tool.exe", "/quiet"]);
    // Nothing there; not a program; an uninstaller.
    let gone = Link { link_info_path: Some(wide("C:\\Gone\\gone.exe")), ..Link::default() };
    assert!(apps::entry_for("/nt/c/Users/Public/Desktop/Gone.lnk", &gone, &d).unwrap_err().contains("is not there"));
    let doc = Link { link_info_path: Some(wide("C:\\Tools\\readme.txt")), ..Link::default() };
    assert!(apps::entry_for("/nt/c/Users/Public/Desktop/Readme.lnk", &doc, &d).unwrap_err().contains("not a program"));
    let un = Link { link_info_path: Some(wide("C:\\Tools\\Caf\u{e9}\\tool.exe")), ..Link::default() };
    assert!(apps::entry_for("/nt/c/x/Uninstall Tool.lnk", &un, &d).is_err());
}

#[test]
fn the_same_app_from_the_start_menu_and_the_desktop_is_one_entry() {
    let d = disk();
    let a = browser_entry(&d);
    let mut b = a.clone();
    b.link = "/nt/c/Users/Public/Desktop/Example Browser.lnk".into();
    let mut c = a.clone();
    c.argv.push("--incognito".into());
    let out = apps::dedup(alloc::vec![a.clone(), b, c]);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].link, a.link);
    assert_eq!(out[1].id, "poly-windows-example-browser-2.desktop");
}

fn reg_sz(name: &str, value: &str) -> poly_registry::tree::Value {
    poly_registry::tree::Value {
        name: name.encode_utf16().collect(),
        kind: 1,
        data: value.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect(),
    }
}

fn key(
    name: &str,
    values: Vec<poly_registry::tree::Value>,
    subkeys: Vec<poly_registry::hive::KeyImage>,
) -> poly_registry::hive::KeyImage {
    poly_registry::hive::KeyImage { name: name.encode_utf16().collect(), values, subkeys, ..Default::default() }
}

fn path_of(names: &[&str], leaf: poly_registry::hive::KeyImage) -> poly_registry::hive::KeyImage {
    names.iter().rev().fold(leaf, |k, n| key(n, Vec::new(), alloc::vec![k]))
}

fn software_hive() -> Vec<u8> {
    let browser = key(
        "Example Browser",
        alloc::vec![
            reg_sz("DisplayName", "Example Browser"),
            reg_sz("DisplayVersion", "157.0.8103.38"),
            reg_sz("Publisher", "Example Corp"),
            reg_sz(
                "UninstallString",
                "\"C:\\Program Files\\Example\\Browser\\Application\\157.0\\setup.exe\" --uninstall --system-level"
            ),
            reg_sz("DisplayIcon", "C:\\Program Files\\Example\\Browser\\Application\\browser.exe,0"),
        ],
        Vec::new(),
    );
    let hidden = key(
        "{HIDDEN}",
        alloc::vec![
            reg_sz("DisplayName", "Runtime"),
            poly_registry::tree::Value {
                name: "SystemComponent".encode_utf16().collect(),
                kind: 4,
                data: 1u32.to_le_bytes().to_vec()
            }
        ],
        Vec::new(),
    );
    let nameless = key("{NONAME}", Vec::new(), Vec::new());
    let uninstall = key("Uninstall", Vec::new(), alloc::vec![browser, hidden, nameless]);
    let editor = key(
        "Editor",
        alloc::vec![
            reg_sz("DisplayName", "Editor 2"),
            reg_sz("InstallLocation", "C:\\Program Files\\Editor\\"),
            reg_sz("UninstallString", "C:\\Program Files\\Editor\\editor.exe /uninstall"),
        ],
        Vec::new(),
    );
    let wow = path_of(
        &["WOW6432Node", "Microsoft", "Windows", "CurrentVersion"],
        key("Uninstall", Vec::new(), alloc::vec![editor]),
    );
    let native = path_of(&["Microsoft", "Windows", "CurrentVersion"], uninstall);
    let root = key("SOFTWARE", Vec::new(), alloc::vec![native, wow]);
    poly_registry::hive::encode(&poly_registry::hive::HiveFile { sequence: 3, root })
}

fn user_hive() -> Vec<u8> {
    let tool = key(
        "Tool",
        alloc::vec![reg_sz("DisplayName", "Tool"), reg_sz("UninstallString", "C:\\Tools\\Caf\u{e9}\\tool.exe /x")],
        Vec::new(),
    );
    let root = path_of(
        &["Software", "Microsoft", "Windows", "CurrentVersion"],
        key("Uninstall", Vec::new(), alloc::vec![tool]),
    );
    let root = key("USER", Vec::new(), alloc::vec![root]);
    poly_registry::hive::encode(&poly_registry::hive::HiveFile { sequence: 1, root })
}

#[test]
fn uninstall_keys_are_read_from_the_hive_files() {
    let all = apps::uninstall_entries(Some(&software_hive()), Some(&user_hive()));
    let names: Vec<(&str, &str)> = all.iter().map(|u| (u.view, u.display_name.as_str())).collect();
    assert_eq!(names, [("HKLM", "Example Browser"), ("HKLM (32-bit)", "Editor 2"), ("HKCU", "Tool")]);
    assert_eq!(all[0].publisher, "Example Corp");
    assert_eq!(all[0].display_version, "157.0.8103.38");
    // A hive that cannot be read is no hive.
    assert_eq!(apps::uninstall_entries(Some(b"garbage"), None), Vec::<Uninstall>::new());
    assert_eq!(apps::uninstall_entries(None, None), Vec::<Uninstall>::new());
    // What an installer made: what was not there before.
    let before = apps::uninstall_entries(None, Some(&user_hive()));
    let made = apps::new_uninstall_entries(&before, all);
    assert_eq!(made.len(), 2);
}

#[test]
fn an_entry_finds_its_apps_uninstall_key_and_the_desktop_file_carries_it() {
    let d = disk();
    let all = apps::uninstall_entries(Some(&software_hive()), Some(&user_hive()));
    let e = browser_entry(&d);
    let r = apps::registration_for(&e.target, &all).unwrap();
    assert_eq!(r.key, "Example Browser");
    assert_eq!(apps::registration_for("C:\\Program Files\\Editor\\editor.exe", &all).unwrap().key, "Editor");
    assert_eq!(apps::registration_for("C:\\Tools\\Caf\u{e9}\\tool.exe", &all).unwrap().key, "Tool");
    assert_eq!(apps::registration_for("C:\\Elsewhere\\x.exe", &all), None);
    // With one key made, that one.
    assert_eq!(apps::registration_for("C:\\Elsewhere\\x.exe", &all[2..]).unwrap().key, "Tool");
    let text = apps::desktop_file(&e, None, Some(r));
    assert!(text.contains("X-Poly-Windows-Key=HKLM\\Example Browser\n"), "{text}");
    assert!(text.contains("X-Poly-Windows-Publisher=Example Corp\n"));
    assert!(text.contains("X-Poly-Windows-Version=157.0.8103.38\n"));
    assert_eq!(
        apps::desktop_value(&text, apps::UNINSTALL_KEY),
        Some("\"C:\\Program Files\\Example\\Browser\\Application\\157.0\\setup.exe\" --uninstall --system-level")
    );
}

#[test]
fn an_uninstall_string_becomes_a_command() {
    let d = disk();
    assert_eq!(
        apps::uninstall_argv(
            "\"C:\\Program Files\\Example\\Browser\\Application\\157.0\\setup.exe\" --uninstall --system-level",
            &d
        )
        .unwrap(),
        ["/nt/c/Program Files/Example/Browser/Application/157.0/setup.exe", "--uninstall", "--system-level"]
    );
    // Unquoted, with spaces in the path.
    assert_eq!(
        apps::uninstall_argv("C:\\Program Files\\Editor\\editor.exe /uninstall \"x y\"", &d).unwrap(),
        ["/nt/c/Program Files/Editor/editor.exe", "/uninstall", "x y"]
    );
    assert_eq!(apps::uninstall_argv("MsiExec.exe /X{1234}", &d), None);
    assert_eq!(apps::uninstall_argv("", &d), None);
}

#[test]
fn new_and_changed_shortcuts_are_what_the_installer_made() {
    let s = |p: &str, sum: u32| Seen { path: p.into(), sum };
    let before = [s("/a.lnk", 1), s("/b.lnk", 2)];
    let after = [s("/a.lnk", 1), s("/b.lnk", 3), s("/c.lnk", 4)];
    assert_eq!(apps::changed(&before, &after), ["/b.lnk", "/c.lnk"]);
    assert_ne!(apps::checksum(b"x"), apps::checksum(b"y"));
    let folders = apps::link_folders();
    assert_eq!(folders[0], "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs");
    assert_eq!(folders[1], "C:\\Users\\poly\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs");
}
