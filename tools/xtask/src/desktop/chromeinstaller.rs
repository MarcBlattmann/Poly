//! Boot group `chrome-installer` (batch B7 of `docs/windows/chrome-installer-plan.md`): Google's
//! own `ChromeSetup.exe`, unchanged, opened from the Downloads folder through Poly's front door
//! (ADR 0230), as the owner installs Chrome.
//!
//! Poly never ships Google's installer (plan, section 4): the host downloads it into
//! `target/chrome-installer/` beside the other caches (or takes `POLY_CHROME_SETUP=FILE`), and the
//! test is skipped when it cannot.
//! The guest has user networking, since the stub downloads Chrome itself. Then:
//!
//! 1. The Terminal runs `poly-installer ~/Downloads/ChromeSetup.exe`, what a double click in
//!    Files runs: the installer window opens.
//! 2. **Install** runs it un-elevated; it relaunches itself with `runas`, and Poly's consent prompt
//!    (ADR 0222) is answered **Yes** (clicked: No has the focus, as on Windows).
//! 3. The elevated copy unpacks Google's updater, which installs its services (B1), serves COM
//!    (B2) and registers its task (B3), downloads Chrome and runs Chrome's `setup.exe` (B4).
//! 4. Chrome's shortcut becomes a launcher entry; the launcher starts Chrome and it browses.
//! 5. What Google's installer left: Chrome's files, the updater's services (keys in the `SYSTEM`
//!    hive) and its scheduled task (a file in `C:\Windows\System32\Tasks`), counted in the guest.
//! 6. **Uninstall** (ADR 0230; asked on the compositor's control pipe, as Files' Applications
//!    asks): the installer window runs Chrome's `UninstallString`; then the same counts are all zero.
//!
//! Every milestone is waited for with a long timeout, with a screenshot every half minute while
//! nothing happens, so a run that stops shows where.

use super::{
    Boot, Driver, Lines, READY, Result, Symbols, boot_with, chord, click, devices, expect, root, screen, shots_dir,
    state_drive, type_str, wait_line,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Where the installer is in the guest, as a download leaves it.
const DOWNLOAD: &str = "/root/Downloads/ChromeSetup.exe";
/// Google's current stub installer (32-bit). Downloaded on the host, never committed.
const URL: &str = "https://dl.google.com/chrome/install/latest/chrome_installer.exe";
/// The disk with `C:` and the registry: Chrome and its updater need room.
const DISK: &str = "poly-chrome-installer-disk.img";
const SPARE_MB: u64 = 4096;
/// Chrome's launcher entry, from the shortcut its setup writes into the common Start menu.
const APP: &str = "Google Chrome";
/// The consent prompt's Yes button (ADR 0222's prompt, centred on the 1280x800 screen).
const YES: (i32, i32) = (719, 457);
/// Guest memory: the installer, the updater's processes, then Chrome.
const MEMORY: &str = "4G";

/// Where `C:` and the registry's hive files are in the guest (the disk on `/mnt`, ADR 0066).
const C: &str = "/mnt/poly/nt/c";
const HIVES: &str = "/mnt/poly/nt/registry";

/// The Terminal prints what is left of Chrome's install on the serial line:
/// `b7-state: <what> chrome=N services=N tasks=N`.
fn report_state(d: &mut Driver, what: &str) -> std::result::Result<(usize, usize, usize), String> {
    let marker = format!("b7-state: {what} ");
    let cmd = format!(
        "echo \"{marker}chrome=$(ls '{C}/Program Files/Google/Chrome/Application/chrome.exe' 2>/dev/null | wc -l) \
         services=$(tr -d '\\000' < {HIVES}/SYSTEM.hive | grep -o 'GoogleUpdater[A-Za-z]*Service' | sort -u | wc -l) \
         tasks=$(ls {C}/Windows/System32/Tasks 2>/dev/null | grep -ci google)\" > /dev/console\n"
    );
    let n = super::count(&d.lines, &marker) + 1;
    type_str(d, &cmd)?;
    let line = super::wait_nth_line(&d.lines, &marker, n, 60)?;
    let num = |key: &str| -> usize {
        line.split(&format!("{key}="))
            .nth(1)
            .and_then(|r| r.split_whitespace().next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    };
    Ok((num("chrome"), num("services"), num("tasks")))
}

/// When the install fails: Google's updater logs to `updater.log` (a failed check's message
/// too); a new Terminal prints its end on the serial line (`b7-log:` lines), since the disk image
/// is not readable afterwards.
fn dump_logs(d: &mut Driver) {
    let _ = (|| -> std::result::Result<(), String> {
        chord(d, &["ctrl", "spc"])?;
        d.wait(super::LAUNCHER_SHOWN, 30)?;
        type_str(d, "terminal")?;
        thread::sleep(Duration::from_millis(800));
        d.type_text(&["ret"])?;
        thread::sleep(Duration::from_secs(4));
        let cmd = format!(
            "for f in '{C}/Program Files (x86)/Google/GoogleUpdater/updater.log' {C}/Windows/SystemTemp/*/updater.log; \
             do tail -n 60 \"$f\" 2>/dev/null | sed 's/^/b7-log: /'; done > /dev/console; echo b7-log-end > /dev/console\n"
        );
        type_str(d, &cmd)?;
        d.wait("b7-log-end", 60)?;
        Ok(())
    })();
}

/// The cached installer on the host (downloaded once; `POLY_CHROME_SETUP` overrides).
fn installer() -> std::result::Result<PathBuf, String> {
    if let Ok(p) = std::env::var("POLY_CHROME_SETUP") {
        return Ok(PathBuf::from(p));
    }
    let ntdll = crate::ntdll::cache_dir(&root());
    let dir = ntdll.parent().map_or_else(|| root().join("target"), Path::to_path_buf).join("chrome-installer");
    let file = dir.join("ChromeSetup.exe");
    if file.is_file() {
        return Ok(file);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = dir.join("ChromeSetup.exe.part");
    let got = std::process::Command::new("curl")
        .args(["-sSL", "--fail", "--max-time", "600", "-o"])
        .arg(&part)
        .arg(URL)
        .status()
        .map_err(|e| format!("curl: {e}"))?;
    if !got.success() {
        return Err(format!("downloading {URL} failed ({got})"));
    }
    std::fs::rename(&part, &file).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(file)
}

/// Wait for `text`, taking a screenshot every 30 s meanwhile (named after `what`).
fn wait_watching(d: &mut Driver, text: &str, secs: u64, what: &str) -> std::result::Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut n = 0;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match wait_line(&d.lines, text, left.min(Duration::from_secs(30))) {
            Ok(l) => return Ok(l),
            // The front door says the installer ended without an app: nothing more will come.
            Err(_)
                if super::count(&d.lines, "installed no app") + super::count(&d.lines, ": no launcher entry") > 0 =>
            {
                let _ = d.shot(&format!("{what}-ended"));
                return Err(format!("no line {text:?}: the installer ended without installing an app"));
            }
            Err(e) if left <= Duration::from_secs(30) => return Err(e),
            Err(_) => {
                n += 1;
                let _ = d.shot(&format!("{what}-{n:02}"));
            }
        }
    }
}

fn steps(d: &mut Driver) -> std::result::Result<(), String> {
    thread::sleep(Duration::from_millis(500));

    // 1. Open the download the way Files does.
    type_str(d, "terminal")?;
    d.wait("launcher query \"terminal\"", 30)?;
    thread::sleep(Duration::from_millis(500));
    d.type_text(&["ret"])?;
    d.wait("poly-compositor: launched Terminal", 30)?;
    super::shown(d, "Terminal", 90)?;
    d.wait("poly-terminal: ", 30)?;
    thread::sleep(Duration::from_secs(3));
    // Nothing of Chrome's is there yet (this also checks the counting the end relies on).
    let before = report_state(d, "before")?;
    expect(&format!("a clean machine (chrome, services, tasks: {before:?})"), before == (0, 0, 0))?;
    type_str(d, &format!("poly-installer {DOWNLOAD}\n"))?;
    let said = d.wait(&format!("poly-installer: {DOWNLOAD}: Windows"), 90)?;
    expect(&format!("the front door knows Google's installer ({said:?})"), said.contains("Windows"))?;
    let line = d.wait("window \"Install ", 90)?;
    let place = super::window_place(&line).ok_or_else(|| format!("unreadable window line {line:?}"))?;
    thread::sleep(Duration::from_secs(2));
    d.shot("chrome-installer-ready")?;

    // 2. Install, and Yes on the consent prompt.
    let (x, y) = crate::installer::main_button(place);
    click(d, x, y)?;
    d.wait(&format!("poly-installer: running {DOWNLOAD}"), 60)?;
    let asked = wait_watching(d, "poly-compositor: consent prompt", 300, "chrome-installer-before-prompt")?;
    expect(&format!("the prompt names Google's installer ({asked:?})"), asked.contains("ChromeSetup"))?;
    thread::sleep(Duration::from_secs(2));
    d.shot("chrome-installer-prompt")?;
    // No has the focus, as on Windows: Yes is clicked (the prompt is centred on the 1280x800 screen).
    click(d, YES.0, YES.1)?;
    let answer = d.wait("poly-compositor: consent 1: ", 30)?;
    expect(&format!("the prompt was answered Yes ({answer:?})"), answer.ends_with("yes"))?;
    d.shot("chrome-installer-after-yes")?;

    // 3. Google's updater installs Chrome: its shortcut becomes a launcher entry.
    let entry = wait_watching(d, "poly-installer: launcher entry ", 1800, "chrome-installer-installing")?;
    expect(&format!("Chrome's shortcut became a launcher entry ({entry:?})"), entry.contains("chrome.exe"))?;
    // `poly-installer: launcher entry <ID> for ...`
    let id = entry
        .split("launcher entry ")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .ok_or_else(|| format!("no entry id in {entry:?}"))?
        .to_string();
    d.wait("app(s) in the launcher (read again)", 120)?;
    thread::sleep(Duration::from_secs(2));
    d.shot("chrome-installer-done")?;

    // 4. The launcher starts Chrome.
    chord(d, &["ctrl", "spc"])?;
    d.wait(super::LAUNCHER_SHOWN, 30)?;
    type_str(d, "chrome")?;
    d.wait("launcher query \"chrome\"", 30)?;
    thread::sleep(Duration::from_secs(1));
    d.shot("chrome-installer-launcher")?;
    d.type_text(&["ret"])?;
    d.wait(&format!("poly-compositor: launched {APP}"), 60)?;
    wait_watching(d, "window \"New Tab", 600, "chrome-installer-chrome-starting")?;
    thread::sleep(Duration::from_secs(5));
    d.shot("chrome-installer-chrome")?;

    // 5. What the install left, from a new Terminal (Chrome is closed first: Ctrl+Shift+W).
    chord(d, &["ctrl", "shift", "w"])?;
    thread::sleep(Duration::from_secs(5));
    chord(d, &["ctrl", "spc"])?;
    d.wait(super::LAUNCHER_SHOWN, 30)?;
    type_str(d, "terminal")?;
    thread::sleep(Duration::from_millis(800));
    d.type_text(&["ret"])?;
    thread::sleep(Duration::from_secs(3));
    let (chrome, services, tasks) = report_state(d, "installed")?;
    expect(
        &format!(
            "Chrome, the updater's services and its task are there (chrome={chrome} services={services} tasks={tasks})"
        ),
        chrome == 1 && services >= 2 && tasks >= 1,
    )?;

    // 6. Uninstall, as Files' Applications asks the compositor (ADR 0180, 0230).
    type_str(d, &format!("echo 'uninstall {id}' > $POLY_COMPOSITOR_CONTROL\n"))?;
    d.wait(&format!("poly-compositor: asked to uninstall {id}"), 60)?;
    let line = d.wait(&format!("window \"Uninstall {APP}"), 90)?;
    let place = super::window_place(&line).ok_or_else(|| format!("unreadable window line {line:?}"))?;
    thread::sleep(Duration::from_secs(2));
    d.shot("chrome-installer-uninstall-asks")?;
    let (x, y) = crate::installer::main_button(place);
    click(d, x, y)?;
    // Chrome's uninstaller is elevated: Poly's consent prompt again.
    let asked = wait_watching(d, "poly-compositor: consent prompt 2", 300, "chrome-installer-uninstalling")?;
    expect(&format!("the uninstaller asks for consent ({asked:?})"), asked.contains("setup.exe"))?;
    thread::sleep(Duration::from_secs(2));
    click(d, YES.0, YES.1)?;
    d.wait("poly-compositor: consent 2: yes", 30)?;
    let done = wait_watching(d, &format!("poly-installer: uninstalled {APP}: "), 900, "chrome-installer-uninstalling")?;
    expect(&format!("the installer names the entry that went ({done:?})"), done.contains(&id))?;
    d.wait("app(s) in the launcher (read again)", 120)?;
    thread::sleep(Duration::from_secs(2));
    d.shot("chrome-installer-uninstalled")?;
    // Google's updater removes itself once no app is left (it runs `--uninstall-if-unused`); its
    // services and task go with it.
    let deadline = Instant::now() + Duration::from_secs(600);
    loop {
        let (chrome, services, tasks) = report_state(d, "uninstalled")?;
        if chrome == 0 && services == 0 && tasks == 0 {
            break;
        }
        if Instant::now() > deadline {
            return Err(format!(
                "after the uninstall, left: chrome={chrome} services={services} tasks={tasks} (all should be 0)"
            ));
        }
        thread::sleep(Duration::from_secs(30));
    }
    d.shot("chrome-installer-all-gone")?;
    Ok(())
}

fn drive(socket: &Path, lines: Lines) -> std::result::Result<(), String> {
    let shots = shots_dir();
    std::fs::create_dir_all(&shots).map_err(|e| format!("{}: {e}", shots.display()))?;
    let qmp = screen::Qmp::connect(socket, Duration::from_secs(20))?;
    let mut d = Driver { qmp, lines, shots, step: 700 };
    let r = steps(&mut d);
    if r.is_err() {
        let _ = d.shot("chrome-installer-failed");
        dump_logs(&mut d);
    }
    d.qmp.quit();
    r
}

/// The boot test.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    if cfg!(not(unix)) {
        eprintln!("==> skipped: the chrome-installer boot test needs a Unix host (QMP)");
        return Ok(());
    }
    let setup = match installer() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("==> skipped: the chrome-installer boot test: Google's installer: {e}");
            return Ok(());
        }
    };
    let exe = std::fs::read(&setup).map_err(|e| format!("{}: {e}", setup.display()))?;
    let extra = vec![(DOWNLOAD.trim_start_matches('/').to_string(), 0o644, exe)];
    let image = super::image_with("poly-desktop-chrome-installer-initramfs.cpio", false, extra)?;
    let mut tree = crate::rootdisk::RootTree::default();
    tree.dir("poly", 0o755);
    let disk = root().join("target").join(DISK);
    crate::rootdisk::set_spare_mb(SPARE_MB);
    let written = crate::rootdisk::write_image_poly_ext2(&tree, &disk);
    crate::rootdisk::set_spare_mb(0);
    written?;
    eprintln!("==> boot test: Google's Chrome installer from Downloads, through the front door, to Chrome running");
    let socket = std::env::temp_dir().join(format!("poly-chrome-installer-qmp-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let mut qemu_args = vec![
        "-initrd".to_string(),
        image.display().to_string(),
        "-qmp".to_string(),
        format!("unix:{},server=on,wait=off", socket.display()),
    ];
    qemu_args.extend(devices());
    qemu_args.extend(state_drive(&disk));
    qemu_args.extend(crate::net::nic_args(None, false));
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
    let b: Result<Boot> = crate::with_boot_timeout(Duration::from_secs(3000), || {
        boot_with(
            kernel,
            syms,
            MEMORY,
            &super::append(&format!(
                "poly.mount=virtio0:/mnt{}",
                // POLY_B7_TRACE=1: one serial line per system call of the Windows programs (poly.nt.trace),
                // and every exception with its module, offset and 64 stack quadwords.
                if std::env::var_os("POLY_B7_TRACE").is_some() { " poly.nt.trace poly.nt.exceptions=64" } else { "" }
            )),
            crate::Devices::None,
            &qemu_args,
            None,
            on_line,
        )
    });
    let _ = std::fs::remove_file(&socket);
    let b = b?;
    let log = root().join("target").join("chrome-installer.serial.log");
    let _ = std::fs::write(&log, &b.output);
    eprintln!("    serial log: {}", log.display());
    match result.lock().unwrap_or_else(|e| e.into_inner()).take() {
        Some(Ok(())) => Ok(()),
        Some(Err(e)) => {
            for l in b.output.lines().filter(|l| {
                l.contains("poly-installer")
                    || l.contains("consent")
                    || l.contains("not found")
                    || l.contains("stub:")
                    || l.contains("exception")
                    || l.contains("exited")
                    || l.contains("b7-log")
            }) {
                eprintln!("    {l}");
            }
            Err(e)
        }
        None if !b.output.contains(READY) => {
            Err(format!("the desktop never came up (QEMU status {:?}, timed out: {})", b.status, b.timed_out))
        }
        None => Err("the chrome-installer boot test did not finish".into()),
    }
}
