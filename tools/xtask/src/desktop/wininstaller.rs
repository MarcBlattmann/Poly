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
    shown(d, "Terminal", 90)?;
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
    let extra = vec![(DOWNLOAD.trim_start_matches('/').to_string(), 0o644, exe)];
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
