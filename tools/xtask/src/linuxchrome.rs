//! Boot group `linux-chrome`: Google Chrome for Linux installed from its
//! `.deb` by Poly's installer and started from the desktop (ADR 0218; the
//! installer is ADR 0173).
//!
//! Google's package is not in Debian's archive and is not pinned: `cargo
//! xtask chromium fetch-chrome` downloads the current one from Google's
//! direct link into [`deb_path`] (it is never committed or run on the
//! host). The group skips itself where it or the locked Debian packages are
//! not there, and on hosts without Unix sockets (QMP). Then, on Chromium's
//! disk with the package in Downloads:
//!
//! 1. `xdg-open` of the file as user `poly` (what a browser's "open" of a
//!    download runs) brings up the installer; **Install** configures the
//!    base root, fetches the package lists and installs Chrome with its
//!    dependencies from Debian's snapshot archive (the guest's network).
//! 2. `dpkg` has it installed, nothing is left unconfigured, and
//!    `chrome-sandbox` is set-user-ID root.
//! 3. The installer's **Open** starts it the way the launcher does: as user
//!    `poly` (Chrome refuses to run its sandbox as root), and a window shows.
//! 4. Its first start's windows are answered (Terms, Welcome, sign-in)
//!    until its new tab page shows; then a test page in a new window is
//!    drawn: the four colours of `colours.html` fill its quadrants.
//! 5. bash's process substitution, which Chrome's launch script uses
//!    (`exec > >(exec cat)`), works: a pipe's `/dev/fd` link opens the pipe.

use crate::chromium::{self, PolyFiles};
use crate::installer::{STEP_WAIT, WINDOW_WAIT, expect, main_button};
use crate::symbols::Symbols;
use crate::toolkits::{Machine, find_colour, window_place};
use crate::{Result, root, rootdisk};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Google's direct link to the current stable package.
pub const URL: &str = "https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb";
/// The package's name, and its file in Downloads.
const PACKAGE: &str = "google-chrome-stable";
const FILE: &str = "google-chrome-stable_current_amd64.deb";
/// The image the test makes afresh.
const IMAGE: &str = "poly-linux-chrome-test.img";
/// Free room on that image: Chrome and what it needs take about 600 MB.
const SPARE_MB: u64 = 2048;

/// Where `fetch-chrome` keeps the package.
pub fn deb_path() -> PathBuf {
    root().join("target/chromium/google-chrome").join(FILE)
}

/// `cargo xtask chromium fetch-chrome`: download Google Chrome's current
/// `.deb` (again, if there is one: "current" changes).
pub fn fetch() -> Result {
    let to = deb_path();
    std::fs::create_dir_all(to.parent().unwrap_or(Path::new("."))).map_err(|e| format!("{}: {e}", to.display()))?;
    chromium::download(URL, &to)?;
    let size = std::fs::metadata(&to).map(|m| m.len()).unwrap_or(0);
    println!("{} ({} MB)", to.display(), size >> 20);
    Ok(())
}

/// The quadrant colours of `colours.html` (`chromium::PAGES`).
const QUADRANTS: [[u8; 3]; 4] = [[0xff, 0, 0], [0, 0xff, 0], [0, 0, 0xff], [0xff, 0xff, 0]];

/// Wait for a line with one of `needles` after `from`, failing at once if
/// the installer reports a failure: which needle, and the rest of its line.
fn installer_step(m: &mut Machine, from: usize, what: &str) -> Result {
    const ENDS: &str = "poly-installer: failed: ";
    let deadline = std::time::Instant::now() + STEP_WAIT;
    loop {
        let text = m.text();
        let after = text.get(from..).unwrap_or_default();
        if let Some(at) = after.find(ENDS) {
            let tail: String = after[at..].lines().take(40).collect::<Vec<_>>().join("\n");
            return Err(format!("the installer failed before {what:?}:\n{tail}"));
        }
        if after.contains(what) {
            return Ok(());
        }
        if std::time::Instant::now() > deadline {
            return Err(format!("no {what:?} within {} s", STEP_WAIT.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Answer one of Chrome's first-run windows shown at `place` with title
/// line `line`: its Terms (**Accept**), Welcome (**OK**), the sign-in page
/// (**Stay signed out**); the buttons are in the bottom right corner.
fn answer_first_run(m: &mut Machine, line: &str, (x, y, w, h): (i32, i32, i32, i32)) -> Result<bool> {
    let at = if line.contains("Terms of Service") {
        (x + w - 62, y + h - 40)
    } else if line.contains("Welcome to Google Chrome") {
        (x + w - 53, y + h - 39)
    } else if line.contains("window \"Google Chrome\" shown") {
        (x + w - 237, y + h - 43)
    } else {
        return Ok(false);
    };
    m.click(at.0, at.1)?;
    eprintln!("==> answered {line}");
    Ok(true)
}

fn scenario(m: &mut Machine) -> Result {
    let path = format!("/home/poly/Downloads/{FILE}");
    let out = m.run(&format!("echo base-\"$(dpkg -l {PACKAGE} 2>&1 | grep -c '^ii')\""))?;
    expect(&format!("Chrome is not installed yet ({out:?})"), out.contains("base-0"))?;
    // 5. bash's process substitution through a pipe's /dev/fd link.
    // (cat writes after bash has ended, as on Linux: into a file, read
    // back a moment later; the word looked for is not on the command line.)
    let out = m.run(
        "bash -c 'exec > >(exec cat > /tmp/piped.txt); echo piped-\"o\"k' 2>&1; sleep 2; echo got-\"$(cat /tmp/piped.txt)\"",
    )?;
    expect(
        &format!("bash's `exec > >(cat)` works ({out:?})"),
        out.contains("got-piped-ok") && !out.contains("Read-only"),
    )?;
    m.key("esc")?;

    // 1. A browser's open of the download: xdg-open as user poly.
    let from = m.text_len();
    m.send_line(&format!("/usr/local/bin/poly-as-user xdg-open {path} > /tmp/xdg-open.log 2>&1 &"))?;
    m.seen(from, &format!("poly-compositor: asked to open the installer for {path}"), WINDOW_WAIT)?;
    let rest = m.seen(from, &format!("window \"Install {PACKAGE}\""), WINDOW_WAIT)?;
    let place = window_place(&rest).ok_or_else(|| format!("unreadable window line {rest:?}"))?;
    std::thread::sleep(Duration::from_secs(2));
    m.shot("installer")?;
    let from = m.text_len();
    let started = std::time::Instant::now();
    let (x, y) = main_button(place);
    m.click(x, y)?;
    m.seen(from, "poly-installer: running /usr/bin/dpkg", WINDOW_WAIT)?;
    installer_step(m, from, "poly-installer: running /usr/bin/apt-get update")?;
    eprintln!("==> base root configured after {} s", started.elapsed().as_secs());
    installer_step(m, from, "poly-installer: running /usr/bin/apt-get install")?;
    installer_step(m, from, &format!("poly-installer: installed {PACKAGE} "))?;
    eprintln!("==> Google Chrome installed after {} s", started.elapsed().as_secs());
    let done = m.seen(from, &format!("poly-installer: installed {PACKAGE} "), WINDOW_WAIT)?;
    expect(
        &format!("the package brings the launcher entry ({done:?})"),
        done.contains("Google Chrome (google-chrome.desktop)"),
    )?;
    m.seen(from, "app(s) in the launcher (read again)", WINDOW_WAIT)?;
    m.shot("installed")?;

    // 2. dpkg's state; the set-user-ID sandbox helper.
    let out = m.run(&format!(
        "echo chrome-\"$(dpkg -s {PACKAGE} | grep -c '^Status: install ok installed')-$(dpkg --audit | wc -l)-$(stat -c %a /opt/google/chrome/chrome-sandbox)\""
    ))?;
    expect(
        &format!("dpkg has Chrome installed, nothing unconfigured, chrome-sandbox 4755 ({out:?})"),
        out.contains("chrome-1-0-4755"),
    )?;

    // 3. Open, as the launcher would: as user poly, with a window.
    let from = m.text_len();
    m.click(x, y)?;
    m.seen(from, "poly-compositor: asked to open Google Chrome (google-chrome.desktop)", WINDOW_WAIT)?;
    let launched = m.seen(from, "poly-compositor: launched Google Chrome (pid ", WINDOW_WAIT)?;
    let pid: u32 = launched
        .split(')')
        .next()
        .and_then(|p| p.trim().parse().ok())
        .ok_or_else(|| format!("no pid in {launched:?}"))?;
    m.seen(from, "poly-compositor: launch-to-window Google Chrome", WINDOW_WAIT)?;
    m.shot("chrome-from-installer")?;
    let out = m.run(&format!("echo uid-$(awk '/^Uid:/ {{print $2}}' /proc/{pid}/status)"))?;
    expect(&format!("Chrome runs as user poly ({out:?})"), out.contains("uid-1000"))?;

    // Its first start: each first-run window answered once, until its own
    // window, the new tab page.
    let deadline = std::time::Instant::now() + 2 * WINDOW_WAIT;
    let mut answered = 0;
    loop {
        let text = m.text();
        let after = text.get(from..).unwrap_or_default();
        if after.contains("window \"New Tab - Google Chrome\" shown") {
            break;
        }
        let shown: Vec<String> = after
            .lines()
            .filter(|l| l.contains("poly-compositor: window \"") && l.contains(" shown at "))
            .map(String::from)
            .collect();
        if shown.len() > answered {
            answered = shown.len();
            let line = &shown[answered - 1];
            if let Some(place) =
                line.split_once("\" shown at").and_then(|(_, r)| window_place(&format!(" shown at{r}")))
            {
                std::thread::sleep(Duration::from_secs(2));
                if answer_first_run(m, line, place)? {
                    m.shot("chrome-first-run")?;
                }
            }
            continue;
        }
        if std::time::Instant::now() > deadline {
            m.shot("chrome-first-run-stuck")?;
            return Err(format!("Chrome's new tab page did not come within {} s", (2 * WINDOW_WAIT).as_secs()));
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    m.shot("chrome-new-tab")?;

    // 4. A test page in a window of its own, drawn.
    let page = format!("file:///{}/colours.html", chromium::PAGES_DIR);
    let from = m.text_len();
    m.send_line(&format!(
        "/usr/local/bin/poly-as-user /usr/bin/google-chrome-stable --new-window {page} > /tmp/gc.log 2>&1 &"
    ))?;
    let rest = m.seen(from, "window \"Poly Chromium colours - Google Chrome\"", WINDOW_WAIT)?;
    let place = window_place(&rest).ok_or_else(|| format!("unreadable window line {rest:?}"))?;
    let (px, py, pw, ph) = place;
    let deadline = std::time::Instant::now() + WINDOW_WAIT;
    loop {
        let img = m.shot("chrome-page")?;
        // Each colour covers at least a tenth of the window.
        let need = (pw as usize * ph as usize) / 10;
        let found: Vec<usize> =
            QUADRANTS.iter().map(|c| find_colour(&img, *c, 3, (px, py, pw, ph)).map_or(0, |f| f.1)).collect();
        if found.iter().all(|&n| n >= need) {
            eprintln!("==> Chrome drew the test page: {found:?} pixels of its four colours");
            return Ok(());
        }
        if std::time::Instant::now() > deadline {
            return Err(format!("the test page was not drawn: {found:?} pixels of its colours, {need} wanted each"));
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}

/// The boot test.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    if !cfg!(unix) {
        eprintln!("==> skipped: the linux-chrome boot test needs a Unix host (QMP)");
        return Ok(());
    }
    let locked = chromium::lock()?;
    let absent = chromium::missing(&locked, &chromium::BUILD_SETS);
    if !absent.is_empty() {
        eprintln!(
            "==> skipped: the linux-chrome boot test: {} locked Debian packages not downloaded (cargo xtask chromium fetch)",
            absent.len()
        );
        return Ok(());
    }
    let deb = deb_path();
    if !deb.is_file() {
        eprintln!(
            "==> skipped: the linux-chrome boot test: no Google Chrome package (cargo xtask chromium fetch-chrome)"
        );
        return Ok(());
    }
    let files = PolyFiles::default();
    let mut contents = chromium::collect(&files)?;
    contents.tree.dir("home/poly/Downloads", 0o755);
    contents.tree.host_file(&format!("home/poly/Downloads/{FILE}"), 0o644, &deb, 0);
    let image = root().join("target").join(IMAGE);
    rootdisk::set_spare_mb(SPARE_MB);
    rootdisk::write_image_poly_ext2(&contents.tree, &image)?;
    let initrd = chromium::write_initramfs(&files)?;
    eprintln!(
        "==> boot test: Google Chrome's .deb opened, installed by Poly's installer, started as user poly, a page drawn"
    );
    let init = format!("{} poly.net=qemu", chromium::INIT_ARGS);
    let mut m = Machine::boot_with_init_args(
        kernel,
        syms,
        &image,
        &initrd,
        "linux-chrome-test",
        chromium::MEMORY,
        &init,
        chromium::CPUS,
        &crate::net::nic_args(None, false),
    )?;
    m.set_timeouts(Duration::from_secs(1200), Duration::from_secs(3600));
    m.desktop_up()?;
    let result = scenario(&mut m);
    let log = root().join("target/poly-linux-chrome-boot.log");
    let _ = std::fs::write(&log, m.text());
    let _ = m.poweroff();
    let _ = std::fs::remove_file(&image);
    result.map_err(|e| {
        eprintln!("==> the console's output is in {}", log.display());
        format!("Google Chrome for Linux: {e}")
    })
}
