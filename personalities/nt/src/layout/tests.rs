//! Host tests of the layout table, its creation on a real tmpfs, the
//! registry sync and the default environment (ADR 0073).

extern crate std;

use super::create::{self, Installed, create, install_file};
use super::env::{self, Block, Machine};
use super::sync::{self, sync};
use super::*;
use crate::io::VfsRoot;
use crate::io::consts::{FILE_ATTRIBUTE_HIDDEN, STATUS_NOT_A_DIRECTORY};
use crate::io::info::{NT_XATTR, Stored};
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_ntobj::name::wide;
use poly_registry::Registry;
use poly_registry::consts::{REG_EXPAND_SZ, REG_SZ};
use poly_registry::defaults::sz;
use poly_vfs::{At, Clock, Cred, NodeRef, OpenFlags, Timespec, Tmpfs, Vfs};

#[derive(Debug)]
struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Timespec {
        Timespec::new(1_000_000_000, 0)
    }
}

/// A tmpfs VFS with `/nt/c`, and the node behind `C:`.
fn setup() -> (VfsRoot, NodeRef) {
    let vfs: &'static Vfs = Box::leak(Box::new(Vfs::new(Arc::new(Tmpfs::new(Arc::new(FixedClock))))));
    let ctx = vfs.root_context().unwrap();
    let cred = Cred::root();
    vfs.mkdir(&ctx, &cred, At::Cwd, b"/nt", 0o755).unwrap();
    vfs.mkdir(&ctx, &cred, At::Cwd, b"/nt/c", 0o755).unwrap();
    let root = vfs.lookup(&ctx, &cred, At::Cwd, b"/nt/c", true).unwrap();
    (VfsRoot { vfs, ctx, cred }, root)
}

fn exists(vr: &VfsRoot, path: &str) -> bool {
    vr.vfs.lookup(&vr.ctx, &vr.cred, At::Cwd, path.as_bytes(), true).is_ok()
}

fn node(vr: &VfsRoot, path: &str) -> NodeRef {
    vr.vfs.lookup(&vr.ctx, &vr.cred, At::Cwd, path.as_bytes(), true).unwrap()
}

fn registry() -> Registry {
    let mut reg = Registry::new();
    poly_registry::defaults::install(&mut reg, 0, &mut |_| None).unwrap();
    reg
}

// ----- the table

#[test]
fn parents_come_before_children_and_no_two_folders_collide() {
    let paths = dos_paths(&POLY_USER);
    for (i, p) in paths.iter().enumerate() {
        let upper = p.to_uppercase();
        for q in &paths[..i] {
            assert_ne!(q.to_uppercase(), upper, "{p} twice");
        }
        // No folder listed later is an ancestor of this one.
        for q in &paths[i + 1..] {
            assert!(!upper.starts_with(&alloc::format!("{}\\", q.to_uppercase())), "{q} is listed after {p}");
        }
        // Every component is a valid NT name.
        for c in p[3..].split('\\') {
            crate::io::names::check_component(&wide(c)).unwrap_or_else(|_| panic!("{p}: bad component {c}"));
        }
    }
}

#[test]
fn the_well_known_folders_are_where_learn_puts_them() {
    let u = &POLY_USER;
    let at = |id: &str| known_folder(id).unwrap_or_else(|| panic!("{id}")).dos_path(u);
    assert_eq!(at("FOLDERID_Windows"), SYSTEM_ROOT);
    assert_eq!(at("FOLDERID_System"), SYSTEM32);
    assert_eq!(at("FOLDERID_SystemX86"), "C:\\Windows\\SysWOW64");
    assert_eq!(at("FOLDERID_ProgramFiles"), PROGRAM_FILES);
    assert_eq!(at("FOLDERID_ProgramFilesX86"), PROGRAM_FILES_X86);
    assert_eq!(at("FOLDERID_ProgramFilesCommon"), COMMON_FILES);
    assert_eq!(at("FOLDERID_ProgramData"), PROGRAM_DATA);
    assert_eq!(at("FOLDERID_UserProfiles"), USERS);
    assert_eq!(at("FOLDERID_Public"), PUBLIC);
    assert_eq!(at("FOLDERID_Profile"), "C:\\Users\\poly");
    assert_eq!(at("FOLDERID_RoamingAppData"), "C:\\Users\\poly\\AppData\\Roaming");
    assert_eq!(at("FOLDERID_LocalAppData"), "C:\\Users\\poly\\AppData\\Local");
    assert_eq!(at("FOLDERID_Documents"), "C:\\Users\\poly\\Documents");
    assert_eq!(at("FOLDERID_CommonStartup"), "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\StartUp");
    assert!(known_folder("FOLDERID_NoSuchThing").is_none());
    assert_eq!(folder_of_variable("temp").unwrap().dos_path(u), u.temp());
    assert_eq!(folder_of_variable("ALLUSERSPROFILE").unwrap().dos_path(u), PROGRAM_DATA);
    assert_eq!(u.profile(), "C:\\Users\\poly");
    assert_eq!(u.home_path(), "\\Users\\poly");
}

#[test]
fn user_shell_folders_are_stored_below_their_variables() {
    let u = &POLY_USER;
    let by_shell = |name: &str| FOLDERS.iter().find(|f| f.shell == Some(name)).unwrap().unexpanded(u);
    assert_eq!(by_shell("Personal"), "%USERPROFILE%\\Documents");
    assert_eq!(by_shell("AppData"), "%USERPROFILE%\\AppData\\Roaming");
    assert_eq!(by_shell("Common AppData"), "%ProgramData%");
    assert_eq!(by_shell("Common Documents"), "%PUBLIC%\\Documents");
    assert_eq!(by_shell("Common Programs"), "%ProgramData%\\Microsoft\\Windows\\Start Menu\\Programs");
    // Every one expands back to the folder.
    let b = sync::expansion_block(u);
    for f in FOLDERS {
        assert_eq!(b.expand(&f.unexpanded(u)), f.dos_path(u), "{}", f.path);
    }
}

#[test]
fn vfs_paths_map_to_c_or_z() {
    let roots: &[&[u8]] = &[b"/mnt/root/poly/nt/c", b"/nt/c/"];
    assert_eq!(dos_path_of_vfs(b"/nt/c/Program Files/7-Zip/7z.exe", roots), "C:\\Program Files\\7-Zip\\7z.exe");
    assert_eq!(dos_path_of_vfs(b"/mnt/root/poly/nt/c/x.exe", roots), "C:\\x.exe");
    assert_eq!(dos_path_of_vfs(b"/nt/c", roots), "C:\\");
    assert_eq!(dos_path_of_vfs(b"/nt/cc/x.exe", roots), "Z:\\nt\\cc\\x.exe");
    assert_eq!(dos_path_of_vfs(b"/bin/test.exe", roots), "Z:\\bin\\test.exe");
    assert_eq!(dos_path_of_vfs(b"rel/a.exe", roots), "Z:\\rel\\a.exe");
}

// ----- creation

#[test]
fn creation_makes_every_folder_once() {
    let (vr, root) = setup();
    let first = create(&vr, &root, &POLY_USER);
    assert!(first.conflicts.is_empty(), "{:?}", first.conflicts);
    assert_eq!(first.created.len(), FOLDERS.len() + 4, "the intermediate Microsoft\\Windows folders count too");
    for f in FOLDERS {
        let p = alloc::format!("/nt/c/{}", f.below_drive(&POLY_USER).replace('\\', "/"));
        assert!(exists(&vr, &p), "{p}");
    }
    // Idempotent.
    let second = create(&vr, &root, &POLY_USER);
    assert_eq!(second, create::Report::default());
    // Hidden where the table says so, and only there.
    let hidden = |p: &str| {
        node(&vr, p).getxattr(&vr.cred, NT_XATTR).map(|v| Stored::parse(&v).attributes & FILE_ATTRIBUTE_HIDDEN != 0)
    };
    assert_eq!(hidden("/nt/c/ProgramData"), Ok(true));
    assert_eq!(hidden("/nt/c/Users/poly/AppData"), Ok(true));
    assert_eq!(hidden("/nt/c/Users/Default"), Ok(true));
    assert_ne!(hidden("/nt/c/Windows"), Ok(true));
    assert_ne!(hidden("/nt/c/Users/poly/AppData/Local"), Ok(true));
}

#[test]
fn existing_folders_are_found_without_case() {
    let (vr, root) = setup();
    // A Linux program made `windows` and `program files/COMMON FILES`.
    vr.vfs.mkdir(&vr.ctx, &vr.cred, At::Cwd, b"/nt/c/windows", 0o700).unwrap();
    vr.vfs.mkdir(&vr.ctx, &vr.cred, At::Cwd, b"/nt/c/program files", 0o700).unwrap();
    vr.vfs.mkdir(&vr.ctx, &vr.cred, At::Cwd, b"/nt/c/program files/COMMON FILES", 0o700).unwrap();
    let report = create(&vr, &root, &POLY_USER);
    assert!(report.conflicts.is_empty());
    assert!(!report.created.iter().any(|p| p == "C:\\Windows" || p == "C:\\Program Files"));
    let top = create::names(&vr, &root).unwrap();
    assert!(top.contains(&"windows".to_string()) && !top.contains(&"Windows".to_string()), "{top:?}");
    assert!(exists(&vr, "/nt/c/windows/System32"));
    let pf = create::names(&vr, &node(&vr, "/nt/c/program files")).unwrap();
    assert_eq!(pf, alloc::vec!["COMMON FILES".to_string()]);
    // The existing folder keeps its own mode: Poly does not touch it.
    assert_eq!(node(&vr, "/nt/c/windows").stat().unwrap().mode & 0o777, 0o700);
}

#[test]
fn names_that_differ_only_in_case_use_the_one_nt_opens() {
    let (vr, root) = setup();
    vr.vfs.mkdir(&vr.ctx, &vr.cred, At::Cwd, b"/nt/c/windows", 0o755).unwrap();
    vr.vfs.mkdir(&vr.ctx, &vr.cred, At::Cwd, b"/nt/c/WINDOWS", 0o755).unwrap();
    let report = create(&vr, &root, &POLY_USER);
    assert!(report.conflicts.is_empty());
    // Neither is `Windows` exactly, so the smallest as bytes wins: `WINDOWS`.
    assert!(exists(&vr, "/nt/c/WINDOWS/System32"));
    assert!(!exists(&vr, "/nt/c/windows/System32"));
    assert!(!exists(&vr, "/nt/c/Windows"));
    // An exact match wins over a smaller one.
    let (vr2, root2) = setup();
    vr2.vfs.mkdir(&vr2.ctx, &vr2.cred, At::Cwd, b"/nt/c/USERS", 0o755).unwrap();
    vr2.vfs.mkdir(&vr2.ctx, &vr2.cred, At::Cwd, b"/nt/c/Users", 0o755).unwrap();
    create(&vr2, &root2, &POLY_USER);
    assert!(exists(&vr2, "/nt/c/Users/poly"));
    assert!(!exists(&vr2, "/nt/c/USERS/poly"));
}

#[test]
fn a_file_in_the_way_is_reported_and_left_alone() {
    let (vr, root) = setup();
    let f = vr
        .vfs
        .open(&vr.ctx, &vr.cred, At::Cwd, b"/nt/c/programdata", OpenFlags::WRONLY | OpenFlags::CREAT, 0o644)
        .unwrap();
    f.write_at(0, b"mine").unwrap();
    drop(f);
    let report = create(&vr, &root, &POLY_USER);
    assert_eq!(report.conflicts, alloc::vec![(PROGRAM_DATA.to_string(), STATUS_NOT_A_DIRECTORY)]);
    assert!(node(&vr, "/nt/c/programdata").stat().unwrap().size == 4);
    // Everything else is there.
    assert!(exists(&vr, "/nt/c/Users/poly/AppData/Local/Temp"));
    assert!(exists(&vr, "/nt/c/Windows/System32"));
}

#[test]
fn system_files_are_installed_and_restored() {
    let (vr, root) = setup();
    create(&vr, &root, &POLY_USER);
    let dir = "Windows\\System32";
    assert_eq!(install_file(&vr, &root, dir, "ntdll.dll", b"MZ one"), Ok(Installed::Written));
    assert_eq!(install_file(&vr, &root, dir, "NTDLL.DLL", b"MZ one"), Ok(Installed::Unchanged));
    // Something changed it: the next boot writes it again, under its own name.
    assert_eq!(install_file(&vr, &root, dir, "ntdll.dll", b"MZ two!"), Ok(Installed::Written));
    let names = create::names(&vr, &node(&vr, "/nt/c/Windows/System32")).unwrap();
    assert_eq!(names.iter().filter(|n| n.eq_ignore_ascii_case("ntdll.dll")).collect::<Vec<_>>(), ["ntdll.dll"]);
    assert_eq!(node(&vr, "/nt/c/Windows/System32/ntdll.dll").stat().unwrap().size, 7);
    // A longer file with the same start is different too.
    assert_eq!(install_file(&vr, &root, dir, "ntdll.dll", b"MZ two"), Ok(Installed::Written));
    assert_eq!(install_file(&vr, &root, dir, "ntdll.dll", b"MZ two"), Ok(Installed::Unchanged));
    // A directory of that name is not replaced.
    assert_eq!(
        install_file(&vr, &root, "Windows", "system32", b"x"),
        Err(poly_ntobj::status::STATUS_OBJECT_NAME_COLLISION)
    );
    // The folder is made when missing, in any case the caller names it.
    assert_eq!(install_file(&vr, &root, "windows\\SYSTEM32\\New", "a.dll", b"a"), Ok(Installed::Written));
    assert!(exists(&vr, "/nt/c/Windows/System32/New/a.dll"));
}

// ----- registry

fn value(reg: &Registry, start: poly_registry::tree::KeyId, path: &str, name: &str) -> Option<(u32, String)> {
    env::string_value(reg, start, path, name)
}

#[test]
fn sync_adds_what_the_layout_needs_and_agrees_with_the_defaults() {
    let mut reg = registry();
    let changed = sync(&mut reg, &POLY_USER, 1).unwrap();
    // What the default hives already hold is not rewritten: they agree.
    for agreed in [
        "ProgramFilesDir",
        "ProgramW6432Dir",
        "ProgramFilesDir (x86)",
        "CommonFilesDir",
        "SystemRoot",
        "Volatile Environment\\USERPROFILE",
        "Volatile Environment\\APPDATA",
        "Volatile Environment\\LOCALAPPDATA",
        "Volatile Environment\\HOMEPATH",
        "Environment\\TEMP",
        "Environment\\windir",
    ] {
        assert!(!changed.iter().any(|c| c.ends_with(&alloc::format!("\\{agreed}"))), "{agreed} changed: {changed:?}");
    }
    let m = reg.machine();
    let u = reg.user();
    let me = alloc::format!("{}\\{}", env::PROFILE_LIST, POLY_USER.sid);
    assert_eq!(value(&reg, m, &me, "ProfileImagePath"), Some((REG_EXPAND_SZ, "C:\\Users\\poly".into())));
    assert_eq!(
        value(&reg, m, env::PROFILE_LIST, "ProgramData"),
        Some((REG_EXPAND_SZ, "%SystemDrive%\\ProgramData".into()))
    );
    assert_eq!(value(&reg, m, env::CURRENT_VERSION, "CommonW6432Dir"), Some((REG_SZ, COMMON_FILES.into())));
    let usf = alloc::format!("{}\\{}", POLY_USER.sid, sync::USER_SHELL_FOLDERS);
    let sf = alloc::format!("{}\\{}", POLY_USER.sid, sync::SHELL_FOLDERS);
    assert_eq!(value(&reg, u, &usf, "Personal"), Some((REG_EXPAND_SZ, "%USERPROFILE%\\Documents".into())));
    assert_eq!(value(&reg, u, &sf, "Personal"), Some((REG_SZ, "C:\\Users\\poly\\Documents".into())));
    assert_eq!(value(&reg, m, sync::MACHINE_SHELL_FOLDERS, "Common AppData"), Some((REG_SZ, PROGRAM_DATA.into())));
    // The Volatile Environment key stays volatile.
    let vol = reg.find(u, &alloc::format!("{}\\Volatile Environment", POLY_USER.sid)).unwrap();
    assert!(reg.key(vol).unwrap().is_volatile());
}

#[test]
fn a_second_sync_changes_nothing_and_leaves_the_hives_clean() {
    let mut reg = registry();
    sync(&mut reg, &POLY_USER, 1).unwrap();
    for h in reg.dirty_hives() {
        let _ = reg.save(h);
    }
    assert!(reg.dirty_hives().is_empty());
    assert_eq!(sync(&mut reg, &POLY_USER, 2).unwrap(), Vec::<String>::new());
    assert!(reg.dirty_hives().is_empty());
}

#[test]
fn fixed_values_are_restored_and_user_choices_kept() {
    let mut reg = registry();
    sync(&mut reg, &POLY_USER, 1).unwrap();
    let m = reg.machine();
    let u = reg.user();
    // A program broke ProgramFilesDir; the user redirected Documents and TEMP.
    let cv = reg.find(m, env::CURRENT_VERSION).unwrap();
    reg.set_value(cv, &wide("ProgramFilesDir"), REG_SZ, &sz("D:\\Apps"), 2).unwrap();
    let usf = alloc::format!("{}\\{}", POLY_USER.sid, sync::USER_SHELL_FOLDERS);
    let k = reg.find(u, &usf).unwrap();
    reg.set_value(k, &wide("Personal"), REG_EXPAND_SZ, &sz("%USERPROFILE%\\My Stuff"), 2).unwrap();
    let ue = reg.find(u, &alloc::format!("{}\\Environment", POLY_USER.sid)).unwrap();
    reg.set_value(ue, &wide("TEMP"), REG_EXPAND_SZ, &sz("%SystemDrive%\\T"), 2).unwrap();
    let changed = sync(&mut reg, &POLY_USER, 3).unwrap();
    assert!(changed.iter().any(|c| c.ends_with("\\ProgramFilesDir")));
    assert_eq!(value(&reg, m, env::CURRENT_VERSION, "ProgramFilesDir"), Some((REG_SZ, PROGRAM_FILES.into())));
    assert_eq!(value(&reg, u, &usf, "Personal"), Some((REG_EXPAND_SZ, "%USERPROFILE%\\My Stuff".into())));
    let sf = alloc::format!("{}\\{}", POLY_USER.sid, sync::SHELL_FOLDERS);
    assert_eq!(value(&reg, u, &sf, "Personal"), Some((REG_SZ, "C:\\Users\\poly\\My Stuff".into())));
    let e = env::build(&reg, &POLY_USER, Machine { processors: 1 });
    assert_eq!(e.get("TEMP"), Some("C:\\T"));
    assert_eq!(e.get("TMP"), Some("C:\\Users\\poly\\AppData\\Local\\Temp"));
}

// ----- environment

fn default_env() -> Block {
    let mut reg = registry();
    sync(&mut reg, &POLY_USER, 1).unwrap();
    env::build(&reg, &POLY_USER, Machine { processors: 4 })
}

#[test]
fn a_process_gets_every_documented_variable() {
    let e = default_env();
    let want: &[(&str, &str)] = &[
        ("SystemRoot", "C:\\Windows"),
        ("windir", "C:\\Windows"),
        ("SystemDrive", "C:"),
        ("ProgramFiles", "C:\\Program Files"),
        ("ProgramW6432", "C:\\Program Files"),
        ("ProgramFiles(x86)", "C:\\Program Files (x86)"),
        ("CommonProgramFiles", "C:\\Program Files\\Common Files"),
        ("CommonProgramW6432", "C:\\Program Files\\Common Files"),
        ("CommonProgramFiles(x86)", "C:\\Program Files (x86)\\Common Files"),
        ("ProgramData", "C:\\ProgramData"),
        ("ALLUSERSPROFILE", "C:\\ProgramData"),
        ("PUBLIC", "C:\\Users\\Public"),
        ("USERPROFILE", "C:\\Users\\poly"),
        ("HOMEDRIVE", "C:"),
        ("HOMEPATH", "\\Users\\poly"),
        ("APPDATA", "C:\\Users\\poly\\AppData\\Roaming"),
        ("LOCALAPPDATA", "C:\\Users\\poly\\AppData\\Local"),
        ("TEMP", "C:\\Users\\poly\\AppData\\Local\\Temp"),
        ("TMP", "C:\\Users\\poly\\AppData\\Local\\Temp"),
        ("USERNAME", "poly"),
        ("USERDOMAIN", "POLY"),
        ("COMPUTERNAME", "POLY"),
        ("PATH", "C:\\Windows\\system32;C:\\Windows"),
        ("PATHEXT", ".COM;.EXE;.BAT;.CMD"),
        ("OS", "Windows_NT"),
        ("PROCESSOR_ARCHITECTURE", "AMD64"),
        ("NUMBER_OF_PROCESSORS", "4"),
        ("ComSpec", "C:\\Windows\\system32\\cmd.exe"),
    ];
    for (name, value) in want {
        assert_eq!(e.get(name), Some(*value), "{name}");
    }
    // Every variable the table names is its folder.
    for f in FOLDERS {
        for var in f.env {
            assert_eq!(e.get(var), Some(f.dos_path(&POLY_USER).as_str()), "{var}");
        }
    }
}

#[test]
fn the_user_path_is_appended_and_user_variables_override() {
    let mut reg = registry();
    sync(&mut reg, &POLY_USER, 1).unwrap();
    let u = reg.user();
    let ue = reg.find(u, &alloc::format!("{}\\Environment", POLY_USER.sid)).unwrap();
    reg.set_value(ue, &wide("Path"), REG_EXPAND_SZ, &sz("%USERPROFILE%\\bin"), 1).unwrap();
    reg.set_value(ue, &wide("OS"), REG_SZ, &sz("Mine"), 1).unwrap();
    reg.set_value(ue, &wide("Count"), poly_registry::consts::REG_DWORD, &7u32.to_le_bytes(), 1).unwrap();
    let e = env::build(&reg, &POLY_USER, Machine { processors: 0 });
    assert_eq!(e.get("path"), Some("C:\\Windows\\system32;C:\\Windows;C:\\Users\\poly\\bin"));
    assert_eq!(e.get("OS"), Some("Mine"));
    assert_eq!(e.get("Count"), None, "non-string values are skipped");
    assert_eq!(e.get("NUMBER_OF_PROCESSORS"), Some("1"));
}

#[test]
fn an_empty_registry_still_gives_the_layout_values() {
    let reg = Registry::new();
    let e = env::build(&reg, &POLY_USER, Machine { processors: 2 });
    assert_eq!(e.get("SystemRoot"), Some(SYSTEM_ROOT));
    assert_eq!(e.get("ProgramFiles"), Some(PROGRAM_FILES));
    assert_eq!(e.get("USERPROFILE"), Some("C:\\Users\\poly"));
    assert_eq!(e.get("COMPUTERNAME"), Some("POLY"));
}

#[test]
fn the_block_is_sorted_without_case_and_round_trips() {
    let e = default_env();
    let block = e.to_utf16();
    assert_eq!(&block[block.len() - 2..], &[0, 0]);
    let back = Block::from_utf16(&block);
    assert_eq!(back.len(), e.len());
    let names: Vec<&str> = back.sorted().iter().map(|(n, _)| *n).collect();
    let upper: Vec<String> = names.iter().map(|n| n.to_uppercase()).collect();
    let mut sorted = upper.clone();
    sorted.sort();
    assert_eq!(upper, sorted);
    // In the block itself, in that order: `ALLUSERSPROFILE` first, and
    // `CommonProgramFiles` before `CommonProgramFiles(x86)` before
    // `CommonProgramW6432` (`(` is below `W`).
    let text = String::from_utf16(&block).unwrap();
    assert!(text.starts_with("ALLUSERSPROFILE=C:\\ProgramData\0APPDATA="), "{text}");
    let pos = |s: &str| text.find(s).unwrap();
    assert!(
        pos("CommonProgramFiles=") < pos("CommonProgramFiles(x86)=")
            && pos("CommonProgramFiles(x86)=") < pos("CommonProgramW6432=")
    );
    assert_eq!(Block::new().to_utf16(), alloc::vec![0, 0]);
    // A leading `=` belongs to the name.
    let drive: Vec<u16> = "=C:=C:\\x\0A=b=c\0\0junk=1\0".encode_utf16().collect();
    let b = Block::from_utf16(&drive);
    assert_eq!(b.get("=C:"), Some("C:\\x"));
    assert_eq!(b.get("a"), Some("b=c"));
    assert_eq!(b.get("junk"), None);
}

#[test]
fn expansion_follows_expand_environment_strings() {
    let mut b = Block::new();
    b.set("Root", "C:\\Windows");
    b.set("x", "1");
    assert_eq!(b.expand("%ROOT%\\system32"), "C:\\Windows\\system32");
    assert_eq!(b.expand("%nope%\\a"), "%nope%\\a");
    assert_eq!(b.expand("100%"), "100%");
    assert_eq!(b.expand("%%x%%"), "%1%");
    assert_eq!(b.expand("%nope%x%"), "%nope1");
    assert_eq!(b.expand("%x%%x%"), "11");
    assert_eq!(b.expand(""), "");
    b.set("ROOT", "D:");
    assert_eq!(b.get("root"), Some("D:"));
    assert_eq!(b.sorted()[0].0, "Root", "the first name's case stays");
    b.remove("rOOt");
    assert_eq!(b.get("Root"), None);
}
