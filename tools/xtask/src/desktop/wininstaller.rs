//! Boot group `win-installer` (ADR 0230): a Windows installer opened from the
//! Downloads folder through Poly's front door, the way a download is opened.
//!
//! The desktop boots with a small Windows installer in `/root/Downloads`
//! (`tests/windows/win_test_setup.rs`, built by `pipeline win build`) and a
//! disk on `/mnt` with a `poly` folder, so `C:` and the registry's hive files
//! are on a disk (ADR 0066, 0073). Then:
//!
//! 1. The Terminal runs `poly-installer FILE`, what Files and `xdg-open` run
//!    for a `.exe`: the desktop is asked and opens the installer's window,
//!    which says it is a Windows installer (from the PE headers).
//! 2. **Install** runs it on the NT personality, un-elevated. Like Google's
//!    installer it asks for administrator rights itself (`ShellExecuteExW`
//!    `runas`): Poly's consent prompt (ADR 0222) shows, and the test answers
//!    Yes (Tab, Enter). The elevated copy copies itself to
//!    `C:\Program Files`, writes a shortcut into the common Start menu and an
//!    `Uninstall` key, starts a helper that lingers a few seconds, and ends.
//!    The window waits for the helper too (its parent in `/proc`).
//! 3. The shortcut became a launcher entry and the `Uninstall` key its
//!    registration; the desktop read its apps again.
//! 4. **Open** in the installer and the launcher start the installed app
//!    from its entry (the shortcut's target and argument): it says so on its
//!    standard output.
//!
//! 5. A copy of the installer with a `requireAdministrator` manifest embedded (as real installers
//!    carry one; [`with_manifest`]) is opened the same way. The window starts programs as the
//!    Windows shell opens them (`polyopen.exe`, `ShellExecuteExW`, ADR 0230): the manifest is
//!    read and the consent prompt comes **once, before it runs**; it runs elevated, never asks
//!    itself, and registers its app ("Poly Admin App") under `HKLM`.
//!
//! 6. A **Squirrel** installer (ADR 0256; [`squirrel_setup`]: the test program with a `DATA`
//!    resource holding the package bundle) is opened the same way: the window knows it, **Install**
//!    starts Poly's `polysquirrel.exe` instead of the installer, the app lands in
//!    `%LOCALAPPDATA%\PolySquirrelApp\app-1.0.0`, its shortcut becomes a launcher entry, its
//!    per-user `Uninstall` key the registration, and the app starts.
//!
//! Skipped where the Windows test programs cannot be built and on hosts
//! without Unix sockets (QMP).

use super::{
    Boot, Driver, Lines, MEMORY, READY, Result, Symbols, boot_with, chord, click, devices, expect, root, screen,
    shots_dir, shown, state_drive, type_str,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// The test program (`tests/windows/<name>.rs`) and where it is built.
const TEST: &str = "win_test_setup";
const BIN: &str = "target/pipeline/windows/setup-bin";
/// Where it is in the guest, as a download leaves it.
const DOWNLOAD: &str = "/root/Downloads/poly-test-setup.exe";
/// The installer window's heading: the file's name (it has no version
/// resource).
const TITLE: &str = "Install poly-test-setup";
/// The copy with a `requireAdministrator` manifest, its window and its app.
const ADMIN_DOWNLOAD: &str = "/root/Downloads/poly-test-setup-admin.exe";
const ADMIN_TITLE: &str = "Install poly-test-setup-admin";
const ADMIN_APP: &str = "Poly Admin App";
const ADMIN_ID: &str = "poly-windows-poly-admin-app.desktop";

/// An application manifest that asks for administrator rights (Learn, "Application manifests",
/// `requestedExecutionLevel`).
const ADMIN_MANIFEST: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<assembly xmlns=\"urn:schemas-microsoft-com:asm.v1\" manifestVersion=\"1.0\">\r\n\
  <trustInfo xmlns=\"urn:schemas-microsoft-com:asm.v3\">\r\n\
    <security>\r\n\
      <requestedPrivileges>\r\n\
        <requestedExecutionLevel level=\"requireAdministrator\" uiAccess=\"false\"/>\r\n\
      </requestedPrivileges>\r\n\
    </security>\r\n\
  </trustInfo>\r\n\
</assembly>\r\n";

fn u16_at(b: &[u8], at: usize) -> Result<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| "the image is cut short".to_string())
}

fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| "the image is cut short".to_string())
}

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn align(v: u32, a: u32) -> u32 {
    v.div_ceil(a.max(1)) * a.max(1)
}

/// `exe` with `manifest` embedded as its `RT_MANIFEST` resource 1 (language 0x409).
pub fn with_manifest(exe: &[u8], manifest: &str) -> Result<Vec<u8>> {
    with_resource(exe, ResourceType::Id(24), 1, manifest.as_bytes())
}

/// The type of a resource [`with_resource`] embeds.
#[derive(Clone, Copy, Debug)]
pub enum ResourceType {
    Id(u32),
    /// A named type (`DATA`, a Squirrel installer's package bundle).
    Name(&'static str),
}

/// `exe` with `data` embedded as resource `name` of type `kind` (language 0x409): a new last
/// section `.rsrc` holding a resource directory (the PE format specification: the type, name and
/// language tables, one data entry, the data, the type's name string) and the image's resource
/// directory pointing at it. Only for an image without resources and with room for one more
/// section header.
pub fn with_resource(exe: &[u8], kind: ResourceType, name: u32, data: &[u8]) -> Result<Vec<u8>> {
    let pe = u32_at(exe, 0x3C)? as usize;
    if exe.get(..2) != Some(b"MZ") || exe.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err("not a PE image".into());
    }
    let sections = usize::from(u16_at(exe, pe + 6)?);
    let opt_size = usize::from(u16_at(exe, pe + 20)?);
    let opt = pe + 24;
    let (count_at, dirs_at) = match u16_at(exe, opt)? {
        0x20B => (opt + 108, opt + 112),
        0x10B => (opt + 92, opt + 96),
        m => return Err(format!("optional header magic {m:#x}")),
    };
    let (section_align, file_align) = (u32_at(exe, opt + 32)?, u32_at(exe, opt + 36)?);
    let headers = u32_at(exe, opt + 60)? as usize;
    if u32_at(exe, count_at)? < 3 {
        return Err("no resource directory slot".into());
    }
    let res_dir = dirs_at + 2 * 8;
    if u32_at(exe, res_dir)? != 0 {
        return Err("the image has resources already".into());
    }
    let table = opt + opt_size;
    let mut end_va = 0u32;
    let mut first_raw = u32::MAX;
    for i in 0..sections {
        let h = table + i * 40;
        let (vsize, va, raw_size, raw) =
            (u32_at(exe, h + 8)?, u32_at(exe, h + 12)?, u32_at(exe, h + 16)?, u32_at(exe, h + 20)?);
        end_va = end_va.max(va + vsize.max(raw_size));
        if raw_size != 0 {
            first_raw = first_raw.min(raw);
        }
    }
    let new_header = table + sections * 40;
    if new_header + 40 > headers || new_header + 40 > first_raw as usize {
        return Err("no room for another section header".into());
    }
    let va = align(end_va, section_align);
    // The resource directory: root -> type -> name -> language 0x409 -> data entry -> data, then
    // the type's name (its length, UTF-16) for a named type.
    let mut rsrc = Vec::new();
    let dir = |out: &mut Vec<u8>, named: bool, id: u32, target: u32| {
        out.extend_from_slice(&[0u8; 12]);
        out.extend_from_slice(&u16::from(named).to_le_bytes());
        out.extend_from_slice(&u16::from(!named).to_le_bytes());
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&target.to_le_bytes());
    };
    const SUBDIR: u32 = 0x8000_0000;
    let data_at = 88 + data.len() as u32;
    let name_at = data_at.div_ceil(4) * 4;
    match kind {
        ResourceType::Id(id) => dir(&mut rsrc, false, id, SUBDIR | 24),
        ResourceType::Name(_) => dir(&mut rsrc, true, SUBDIR | name_at, SUBDIR | 24),
    }
    dir(&mut rsrc, false, name, SUBDIR | 48);
    dir(&mut rsrc, false, 0x409, 72);
    rsrc.extend_from_slice(&(va + 88).to_le_bytes());
    rsrc.extend_from_slice(&(data.len() as u32).to_le_bytes());
    rsrc.extend_from_slice(&[0u8; 8]);
    rsrc.extend_from_slice(data);
    if let ResourceType::Name(text) = kind {
        rsrc.resize(name_at as usize, 0);
        rsrc.extend_from_slice(&(text.encode_utf16().count() as u16).to_le_bytes());
        for u in text.encode_utf16() {
            rsrc.extend_from_slice(&u.to_le_bytes());
        }
    }
    let len = rsrc.len() as u32;
    let raw = align(exe.len() as u32, file_align);
    let raw_size = align(len, file_align);
    let mut out = exe.to_vec();
    out.resize(raw as usize, 0);
    out.extend_from_slice(&rsrc);
    out.resize((raw + raw_size) as usize, 0);
    let h = new_header;
    out[h..h + 8].copy_from_slice(b".rsrc\0\0\0");
    for (off, v) in [(8, len), (12, va), (16, raw_size), (20, raw), (24, 0), (28, 0), (32, 0), (36, 0x4000_0040)] {
        put32(&mut out, h + off, v);
    }
    out[pe + 6..pe + 8].copy_from_slice(&(sections as u16 + 1).to_le_bytes());
    put32(&mut out, opt + 56, align(va + len, section_align));
    put32(&mut out, res_dir, va);
    put32(&mut out, res_dir + 4, len);
    Ok(out)
}
/// The Squirrel installer (ADR 0256), its window, the app its package carries and its entry.
const SQUIRREL_DOWNLOAD: &str = "/root/Downloads/poly-squirrel-setup.exe";
const SQUIRREL_TITLE: &str = "Install poly-squirrel-setup";
const SQUIRREL_APP: &str = "Poly Squirrel App";
const SQUIRREL_ID: &str = "poly-windows-poly-squirrel-app.desktop";

/// A zip archive of stored files (PKWARE's APPNOTE.TXT: local headers, the central directory, the
/// end record).
fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let p16 = |b: &mut Vec<u8>, v: u16| b.extend_from_slice(&v.to_le_bytes());
    let p32 = |b: &mut Vec<u8>, v: u32| b.extend_from_slice(&v.to_le_bytes());
    for (name, data) in files {
        let at = out.len() as u32;
        let crc = poly_png::crc32(&[data]);
        for (b, in_central) in [(&mut out, false), (&mut central, true)] {
            p32(b, if in_central { 0x0201_4b50 } else { 0x0403_4b50 });
            if in_central {
                p16(b, 20);
            }
            for v in [20, 0, 0, 0, 0] {
                p16(b, v);
            }
            p32(b, crc);
            p32(b, data.len() as u32);
            p32(b, data.len() as u32);
            p16(b, name.len() as u16);
            p16(b, 0);
            if in_central {
                for v in [0, 0, 0] {
                    p16(b, v);
                }
                p32(b, 0);
                p32(b, at);
            }
            b.extend_from_slice(name.as_bytes());
            if !in_central {
                b.extend_from_slice(data);
            }
        }
    }
    let cd_at = out.len() as u32;
    out.extend_from_slice(&central);
    p32(&mut out, 0x0605_4b50);
    for v in [0, 0, files.len() as u16, files.len() as u16] {
        p16(&mut out, v);
    }
    p32(&mut out, central.len() as u32);
    p32(&mut out, cd_at);
    p16(&mut out, 0);
    out
}

/// A Squirrel installer as the boot test needs one: the test program with a `DATA` resource holding
/// the package bundle (`RELEASES`, an `Update.exe` that is never run, the full package), the package
/// a NuGet zip with a `.nuspec` and the app, the test program again, under a name holding `squirrel`.
fn squirrel_setup(exe: &[u8]) -> Result<Vec<u8>> {
    let nuspec = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<package><metadata><id>PolySquirrelApp</id>\
<version>1.0.0</version><title>Poly Squirrel App</title><authors>Poly</authors>\
<description>A test app of Poly's Squirrel installs</description></metadata></package>\n";
    let package = stored_zip(&[
        ("PolySquirrelApp.nuspec", nuspec.as_bytes()),
        ("[Content_Types].xml", b"<Types/>"),
        ("lib/net45/PolySquirrelApp.exe", exe),
    ]);
    let bundle = stored_zip(&[
        ("RELEASES", b"0000000000000000000000000000000000000000 PolySquirrelApp-1.0.0-full.nupkg 1"),
        ("Update.exe", b"MZ, never run"),
        ("PolySquirrelApp-1.0.0-full.nupkg", &package),
    ]);
    with_resource(exe, ResourceType::Name("DATA"), 131, &bundle)
}

/// The launcher entry the shortcut becomes.
const APP: &str = "Poly Test App";
const ID: &str = "poly-windows-poly-test-app.desktop";
/// The disk with `C:` and the registry.
const DISK: &str = "poly-win-installer-disk.img";

/// The test program's path on the host.
fn exe_path() -> PathBuf {
    root().join(BIN).join(format!("{TEST}.exe"))
}

/// Build the test installer.
fn build() -> Result<std::process::ExitStatus> {
    crate::sevenzip::pipeline(&["win", "build", "--tests", TEST, "--out", BIN])
}

/// The prebuild's part: the test installer.
pub fn prebuild() -> Result<Vec<PathBuf>> {
    let built = build()?;
    if !built.success() {
        return Err(format!("building {TEST}.exe failed ({built})"));
    }
    Ok(crate::prebuilt::dir_files(&root().join(BIN)))
}

/// Every step (see the module documentation).
fn drive(socket: &Path, lines: Lines) -> std::result::Result<(), String> {
    let shots = shots_dir();
    std::fs::create_dir_all(&shots).map_err(|e| format!("{}: {e}", shots.display()))?;
    let qmp = screen::Qmp::connect(socket, Duration::from_secs(20))?;
    let mut d = Driver { qmp, lines, shots, step: 300 };
    let r = steps(&mut d);
    if r.is_err() {
        let _ = d.shot("win-installer-failed");
    }
    // QEMU ends with the test, whether it passed or not.
    d.qmp.quit();
    r
}

fn steps(d: &mut Driver) -> std::result::Result<(), String> {
    thread::sleep(Duration::from_millis(500));

    // 1. The Terminal runs what opening a download runs.
    type_str(d, "terminal")?;
    d.wait("launcher query \"terminal\"", 30)?;
    thread::sleep(Duration::from_millis(500));
    d.type_text(&["ret"])?;
    d.wait("poly-compositor: launched Terminal", 30)?;
    let terminal = shown(d, "Terminal", 90)?;
    d.wait("poly-terminal: ", 30)?;
    thread::sleep(Duration::from_secs(3));
    type_str(d, &format!("poly-installer {DOWNLOAD}\n"))?;
    d.wait(&format!("poly-compositor: asked to open the installer for {DOWNLOAD}"), 60)?;
    let said = d.wait(&format!("poly-installer: {DOWNLOAD}: Windows"), 60)?;
    expect(
        &format!("the installer knows a Windows installer by its headers ({said:?})"),
        said.contains("Windows installer (64-bit)"),
    )?;
    let place = shown(d, TITLE, 90)?;
    thread::sleep(Duration::from_secs(2));
    d.shot("win-installer-ready")?;

    // 2. Install: it runs, and the window waits for the helper it started.
    let (x, y) = crate::installer::main_button(place);
    click(d, x, y)?;
    d.wait(&format!("poly-installer: running {DOWNLOAD}"), 60)?;
    // It asks for administrator rights as Google's installer does (`runas`): Poly's consent
    // prompt (ADR 0222). No has the focus first; Tab moves it to Yes, Enter answers.
    let prompt = d.wait("poly-compositor: consent prompt ", 120)?;
    expect(&format!("the prompt names the installer ({prompt:?})"), prompt.contains("poly-test-setup.exe"))?;
    let request = prompt
        .split("consent prompt ")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .ok_or_else(|| format!("unreadable consent line {prompt:?}"))?
        .to_string();
    thread::sleep(Duration::from_millis(1500));
    d.shot("win-installer-consent")?;
    d.type_text(&["tab"])?;
    thread::sleep(Duration::from_millis(300));
    d.type_text(&["ret"])?;
    d.wait(&format!("poly-compositor: consent {request}: yes"), 30)?;
    let waiting = d.wait("poly-installer: the program has ended; waiting for", 120)?;
    expect(
        &format!("the window waits for what the installer started ({waiting:?})"),
        waiting.ends_with("waiting for 1 it started"),
    )?;
    thread::sleep(Duration::from_millis(800));
    d.shot("win-installer-waiting")?;

    // 3. Its shortcut is a launcher entry, its Uninstall key the registration.
    let entry = d.wait(&format!("poly-installer: launcher entry {ID} for "), 120)?;
    expect(
        &format!("the entry runs the shortcut's target ({entry:?})"),
        entry.contains("C:\\Program Files\\Poly Test App\\poly-test-app.exe")
            && entry.contains("Start Menu/Programs/Poly Test App.lnk"),
    )?;
    let registered = d.wait("poly-installer: registered Poly Test App 1.0 ", 30)?;
    expect(
        &format!("the Uninstall key was read from the hive file ({registered:?})"),
        registered.contains(" by Poly: HKLM\\PolyTestApp"),
    )?;
    d.wait(&format!("poly-installer: installed poly-test-setup: {APP} ({ID})"), 30)?;
    d.wait("app(s) in the launcher (read again)", 30)?;
    thread::sleep(Duration::from_secs(2));
    d.shot("win-installer-done")?;

    // 4. Open, from the installer: the app runs from its entry.
    click(d, x, y)?;
    d.wait(&format!("poly-compositor: asked to open {APP} ({ID})"), 60)?;
    d.wait("poly-test-app: started from its shortcut", 120)?;

    // The launcher has it too.
    chord(d, &["ctrl", "spc"])?;
    d.wait(super::LAUNCHER_SHOWN, 30)?;
    type_str(d, "poly test")?;
    d.wait("launcher query \"poly test\"", 30)?;
    thread::sleep(Duration::from_secs(1));
    d.shot("win-installer-launcher")?;
    d.type_text(&["ret"])?;
    d.wait(&format!("poly-compositor: launched {APP}"), 60)?;
    super::wait_nth(&d.lines, "poly-test-app: started from its shortcut", 2, 120)?;

    // 5. The installer whose manifest asks for administrator rights: one prompt, before it runs.
    click(d, terminal.0 + terminal.2 / 2, terminal.1 + terminal.3 / 2)?;
    thread::sleep(Duration::from_secs(1));
    let prompts = super::count(&d.lines, "poly-compositor: consent prompt ");
    type_str(d, &format!("poly-installer {ADMIN_DOWNLOAD}\n"))?;
    d.wait(&format!("poly-compositor: asked to open the installer for {ADMIN_DOWNLOAD}"), 60)?;
    let place = shown(d, ADMIN_TITLE, 90)?;
    thread::sleep(Duration::from_secs(2));
    let (x, y) = crate::installer::main_button(place);
    click(d, x, y)?;
    d.wait(&format!("poly-installer: running {ADMIN_DOWNLOAD}"), 60)?;
    let prompt = super::wait_nth_line(&d.lines, "poly-compositor: consent prompt ", prompts + 1, 120)?;
    expect(
        &format!("the prompt for the manifest names the installer, before it ran ({prompt:?})"),
        prompt.contains("poly-test-setup-admin.exe"),
    )?;
    let request = prompt
        .split("consent prompt ")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .ok_or_else(|| format!("unreadable consent line {prompt:?}"))?
        .to_string();
    thread::sleep(Duration::from_millis(1500));
    d.shot("win-installer-manifest-consent")?;
    d.type_text(&["tab"])?;
    thread::sleep(Duration::from_millis(300));
    d.type_text(&["ret"])?;
    d.wait(&format!("poly-compositor: consent {request}: yes"), 30)?;
    let registered = d.wait(&format!("poly-installer: registered {ADMIN_APP} 1.0 "), 120)?;
    expect(
        &format!("the elevated installer wrote HKLM ({registered:?})"),
        registered.contains(" by Poly: HKLM\\PolyAdminApp"),
    )?;
    d.wait(&format!("poly-installer: installed poly-test-setup-admin: {ADMIN_APP} ({ADMIN_ID})"), 60)?;
    thread::sleep(Duration::from_secs(1));
    d.shot("win-installer-manifest-done")?;
    let asked = super::count(&d.lines, "poly-compositor: consent prompt ") - prompts;
    expect(&format!("the installer with the manifest was asked about once ({asked} prompts)"), asked == 1)?;

    // 6. A Squirrel installer (ADR 0256): Poly's polysquirrel.exe installs its package, the app
    //    starts from its new entry. The Terminal is found by its title bar: the window of step 5
    //    is still open over its middle.
    click(d, terminal.0 + 40, terminal.1 + 8)?;
    thread::sleep(Duration::from_secs(1));
    type_str(d, &format!("poly-installer {SQUIRREL_DOWNLOAD}\n"))?;
    let said = d.wait(&format!("poly-installer: {SQUIRREL_DOWNLOAD}: Windows"), 60)?;
    expect(
        &format!("the installer knows a Squirrel installer by its package ({said:?})"),
        said.contains("Windows installer (Squirrel), installed by Poly"),
    )?;
    let place = shown(d, SQUIRREL_TITLE, 90)?;
    thread::sleep(Duration::from_secs(2));
    let (x, y) = crate::installer::main_button(place);
    click(d, x, y)?;
    // polysquirrel.exe installs it; the installer itself is not run.
    d.wait("poly-installer: running /nt/c/Windows/System32/polysquirrel.exe", 60)?;
    let entry = d.wait(&format!("poly-installer: launcher entry {SQUIRREL_ID} for "), 120)?;
    expect(
        &format!("the entry runs the app in its Squirrel folder ({entry:?})"),
        entry.contains("AppData\\Local\\PolySquirrelApp\\app-1.0.0\\PolySquirrelApp.exe"),
    )?;
    let registered = d.wait(&format!("poly-installer: registered {SQUIRREL_APP} 1.0.0 "), 30)?;
    expect(
        &format!("the per-user Uninstall key names polysquirrel.exe ({registered:?})"),
        registered.contains("polysquirrel.exe\" --uninstall PolySquirrelApp")
            && registered.contains("HKCU\\PolySquirrelApp"),
    )?;
    d.wait(&format!("poly-installer: started {SQUIRREL_APP} ({SQUIRREL_ID})"), 60)?;
    d.wait("poly-squirrel-app: started", 120)?;
    thread::sleep(Duration::from_secs(1));
    d.shot("win-installer-squirrel-done")?;
    Ok(())
}

/// The boot test.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    if cfg!(not(unix)) {
        eprintln!("==> skipped: the win-installer boot test needs a Unix host (QMP)");
        return Ok(());
    }
    let built = if crate::prebuilt::skip("the test Windows installer", &[exe_path()])? {
        Ok(std::process::ExitStatus::default())
    } else {
        build()
    };
    match built {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("==> skipped: the win-installer boot test: building {TEST}.exe failed ({s})");
            return Ok(());
        }
        Err(e) => {
            eprintln!("==> skipped: the win-installer boot test: {e}");
            return Ok(());
        }
    }
    let exe = std::fs::read(exe_path()).map_err(|e| format!("{}: {e}", exe_path().display()))?;
    let admin = with_manifest(&exe, ADMIN_MANIFEST)?;
    let squirrel = squirrel_setup(&exe)?;
    let extra = vec![
        (DOWNLOAD.trim_start_matches('/').to_string(), 0o644, exe),
        (ADMIN_DOWNLOAD.trim_start_matches('/').to_string(), 0o644, admin),
        (SQUIRREL_DOWNLOAD.trim_start_matches('/').to_string(), 0o644, squirrel),
    ];
    let image = super::image_with("poly-desktop-win-installer-initramfs.cpio", false, extra)?;
    // A disk with a `poly` folder: C: and the hive files go on it.
    let mut tree = crate::rootdisk::RootTree::default();
    tree.dir("poly", 0o755);
    let disk = root().join("target").join(DISK);
    crate::rootdisk::write_image_poly_ext2(&tree, &disk)?;
    eprintln!("==> boot test: a Windows installer opened from Downloads, run, its shortcut a launcher entry");
    let socket = std::env::temp_dir().join(format!("poly-win-installer-qmp-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let mut qemu_args = vec![
        "-initrd".to_string(),
        image.display().to_string(),
        "-qmp".to_string(),
        format!("unix:{},server=on,wait=off", socket.display()),
    ];
    qemu_args.extend(devices());
    qemu_args.extend(state_drive(&disk));
    let lines: Lines = Arc::default();
    let result: Arc<Mutex<Option<std::result::Result<(), String>>>> = Arc::default();
    let (seen, outcome, path) = (Arc::clone(&lines), Arc::clone(&result), socket.clone());
    let mut started = false;
    let on_line = move |line: &str| {
        seen.lock().unwrap_or_else(|e| e.into_inner()).push(line.to_string());
        if !started && line.contains(READY) {
            started = true;
            let (lines, outcome, path) = (Arc::clone(&seen), Arc::clone(&outcome), path.clone());
            thread::spawn(move || {
                let r = drive(&path, lines);
                *outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            });
        }
    };
    let b: Result<Boot> = crate::with_boot_timeout(Duration::from_secs(600), || {
        boot_with(
            kernel,
            syms,
            MEMORY,
            &super::append("poly.mount=virtio0:/mnt"),
            crate::Devices::None,
            &qemu_args,
            None,
            on_line,
        )
    });
    let _ = std::fs::remove_file(&socket);
    let b = b?;
    match result.lock().unwrap_or_else(|e| e.into_inner()).take() {
        Some(Ok(())) => {
            for l in b.output.lines().filter(|l| l.contains("poly-installer:")) {
                eprintln!("    {l}");
            }
            Ok(())
        }
        Some(Err(e)) => {
            for l in b.output.lines().filter(|l| {
                l.contains("poly-installer") || l.contains("poly-test") || l.contains("nt:") || l.contains("Poly Test")
            }) {
                eprintln!("    {l}");
            }
            Err(e)
        }
        None if !b.output.contains(READY) => {
            Err(format!("the desktop never came up (QEMU status {:?}, timed out: {})", b.status, b.timed_out))
        }
        None => Err("the win-installer boot test did not finish".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_kernelbase::elevation::{self, Level};

    /// A PE32+ image with one section and no resources (the PE format specification's headers).
    fn image() -> Vec<u8> {
        let mut b = vec![0u8; 0x400];
        b[..2].copy_from_slice(b"MZ");
        put32(&mut b, 0x3C, 0x80);
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
        b[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
        b[0x94..0x96].copy_from_slice(&240u16.to_le_bytes());
        b[0x96..0x98].copy_from_slice(&0x22u16.to_le_bytes());
        let opt = 0x98;
        b[opt..opt + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
        b[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
        put32(&mut b, opt + 32, 0x1000);
        put32(&mut b, opt + 36, 0x200);
        put32(&mut b, opt + 56, 0x2000);
        put32(&mut b, opt + 60, 0x200);
        b[opt + 68..opt + 70].copy_from_slice(&3u16.to_le_bytes());
        put32(&mut b, opt + 108, 16);
        let h = opt + 240;
        b[h..h + 8].copy_from_slice(b".text\0\0\0");
        for (off, v) in [(8, 0x10), (12, 0x1000), (16, 0x200), (20, 0x200), (36, 0x6000_0020)] {
            put32(&mut b, h + off, v);
        }
        b
    }

    #[test]
    fn a_squirrel_setup_carries_its_bundle_in_a_named_data_resource() {
        let setup = squirrel_setup(&image()).unwrap();
        let bundle = poly_squirrel::bundle_resource(&setup).expect("the DATA resource");
        let archive = poly_zip::Archive::parse(bundle).unwrap();
        let b = poly_squirrel::bundle(&archive).expect("a Squirrel bundle");
        assert_eq!((b.id.as_str(), b.version.as_str()), ("PolySquirrelApp", "1.0.0"));
        let package = poly_zip::read(bundle, &b.package, 1 << 20).unwrap();
        let pkg = poly_zip::Archive::parse(&package).unwrap();
        let spec = poly_zip::read(&package, poly_squirrel::spec_entry(&pkg).unwrap(), 4096).unwrap();
        let spec = poly_squirrel::nuspec(core::str::from_utf8(&spec).unwrap()).unwrap();
        assert_eq!((spec.title.as_str(), spec.authors.as_str()), ("Poly Squirrel App", "Poly"));
        assert_eq!(
            poly_zip::read(&package, pkg.find("lib/net45/PolySquirrelApp.exe").unwrap(), 1 << 20).unwrap(),
            image()
        );
    }

    #[test]
    fn the_manifest_is_embedded_and_read_back_as_the_loader_reads_it() {
        let out = with_manifest(&image(), ADMIN_MANIFEST).unwrap();
        let pe = poly_pe::Pe::parse(&out).unwrap();
        assert_eq!(pe.sections().len(), 2);
        // B0's reader (kernelbase's CreateProcessW check) finds the level.
        let mut read = |at: u64, buf: &mut [u8]| -> std::result::Result<usize, u32> {
            let at = at as usize;
            if at >= out.len() {
                return Ok(0);
            }
            let n = buf.len().min(out.len() - at);
            buf[..n].copy_from_slice(&out[at..at + n]);
            Ok(n)
        };
        assert_eq!(elevation::embedded_manifest(&mut read).as_deref(), Some(ADMIN_MANIFEST.as_bytes()));
        assert_eq!(elevation::image_level(&mut read, None), Level::RequireAdministrator);
        // Twice is refused: the image has resources then.
        assert!(with_manifest(&out, ADMIN_MANIFEST).unwrap_err().contains("resources already"));
        assert!(with_manifest(b"not a program at all, but long enough to read e_lfanew from", ADMIN_MANIFEST).is_err());
    }
}
