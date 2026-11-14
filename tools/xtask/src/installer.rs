//! Boot group `linux-installer`: installing a Linux app from a `.deb`, the
//! way a person does it (roadmap Phase 8, "`.deb` packages install with
//! their dependencies and maintainer scripts"; ADR 0132, ADR 0173).
//!
//! The desktop boots from Chromium's disk (`chromium.rs`), which is the
//! Debian base root: Debian's own `dpkg` and `apt` are on it with a
//! database of the packages it is made of, all still unconfigured. A real
//! package from Debian's archive is in the Downloads folder, as a download
//! leaves it: **galculator** (`chromium::INSTALL_SEEDS`), a small GTK 3 app
//! that is not on the disk and depends on a library that is not there
//! either, so `apt` has to fetch one. Then:
//!
//! 1. `xdg-open` of the file as user `poly` (what Chromium's "open" of a
//!    download runs) brings up Poly's installer, started by the desktop;
//!    Cancel closes it.
//! 2. A double click on the file in Files brings it up again. The window
//!    says what the package is.
//! 3. A click on **Install**: `dpkg --configure -a` configures the base
//!    root (once per disk), `apt-get update` fetches the package lists,
//!    `apt-get install` installs the file with its dependency, maintainer
//!    scripts and triggers. The installer reports each step.
//! 4. The desktop has read its apps again: the launcher finds Galculator
//!    without a restart, and Files' Applications lists it.
//! 5. **Open** starts it: its window is shown. So does the launcher.
//!
//! The guest reaches Debian's snapshot archive through QEMU's user
//! networking, as the `chromium` group reaches a public page. The group
//! skips itself where the packages are not fetched (`cargo xtask chromium
//! fetch`) and on hosts without Unix sockets (QMP).

use crate::chromium::{self, PolyFiles};
use crate::symbols::Symbols;
use crate::toolkits::{Machine, window_place};
use crate::{Result, root, rootdisk};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// The image the test makes afresh.
const IMAGE: &str = "poly-installer-test.img";

/// The package the installed app comes from, and what its launcher entry
/// is called.
const PACKAGE: &str = "galculator";
const APP: &str = "Galculator";

/// How long the package tools may take for a step: configuring the base
/// root takes about half a minute with a hypervisor, the downloads depend
/// on the network.
pub(crate) const STEP_WAIT: Duration = Duration::from_secs(900);
/// How long a window may take to show.
pub(crate) const WINDOW_WAIT: Duration = Duration::from_secs(180);

/// The installer's window is `poly_installer_app::view::SIZE`, its buttons
/// 32 high in the bottom right corner behind a margin of 28 (6 lower), the
/// main one 92 wide and the other 10 to its left: where to click, from the
/// window's place.
pub(crate) fn main_button((x, y, w, h): (i32, i32, i32, i32)) -> (i32, i32) {
    (x + w - 28 - 46, y + h - 28 + 6 - 16)
}

pub(crate) fn other_button(place: (i32, i32, i32, i32)) -> (i32, i32) {
    let (x, y) = main_button(place);
    (x - 46 - 10 - 46, y)
}

/// The first row of Files' list: right of the sidebar (224) and the gutter
/// (20), below the header (40), the title (14 + 36 + 6), the column
/// headers (30) and 2, 34 high (`poly_files_app::view::Layout`).
pub(crate) fn first_row((x, y, _, _): (i32, i32, i32, i32)) -> (i32, i32) {
    (x + 224 + 20 + 120, y + 40 + 14 + 36 + 6 + 30 + 2 + 17)
}

pub(crate) fn expect(what: &str, ok: bool) -> Result {
    if ok { Ok(()) } else { Err(format!("not true: {what}")) }
}

/// Wait for the installer's window; its place.
fn installer_window(m: &mut Machine, from: usize) -> Result<(i32, i32, i32, i32)> {
    let rest = m.seen(from, &format!("window \"Install {PACKAGE}\""), WINDOW_WAIT)?;
    window_place(&rest).ok_or_else(|| format!("unreadable window line {rest:?}"))
}

fn scenario(m: &mut Machine, deb: &str) -> Result {
    let path = format!("/home/poly/Downloads/{deb}");
    // 0. The base root as the build made it: Debian's packages unpacked,
    // none configured; the app is not there; the installer is what opens a
    // Debian package, for xdg-utils as for Files.
    // (The answers are told from whatever else the console prints by a
    // word the command line itself does not hold.)
    let out = m.run(&format!(
        "echo base-\"$(dpkg -l | grep -c '^iU')-$(dpkg -l | grep -c '^ii')-$(ls /usr/bin/{PACKAGE} 2>&1 | grep -c 'No such')\"; \
         echo opens-\"$(xdg-mime query default application/vnd.debian.binary-package) $(xdg-mime query filetype {path})\""
    ))?;
    let base = out.lines().find_map(|l| l.trim().strip_prefix("base-")).unwrap_or_default();
    let counts: Vec<u32> = base.split('-').filter_map(|n| n.parse().ok()).collect();
    expect(
        &format!("the disk's packages are unpacked and not configured, the app is not installed ({out:?})"),
        counts.len() == 3 && counts[0] > 200 && counts[1] == 0 && counts[2] == 1,
    )?;
    expect(
        &format!("xdg-mime names the installer for a Debian package ({out:?})"),
        out.contains(&format!("opens-{} application/vnd.debian.binary-package", poly_shell::apps::INSTALLER_ID)),
    )?;
    // The launcher is shown on an empty desktop and has the keyboard.
    m.key("esc")?;

    // 1. Chromium's "open" of a download: xdg-open, as user `poly`. The
    // desktop is asked and starts the installer; Cancel closes it.
    let from = m.text_len();
    m.send_line(&format!("/usr/local/bin/poly-as-user xdg-open {path} > /tmp/xdg-open.log 2>&1 &"))?;
    m.seen(from, &format!("poly-compositor: asked to open the installer for {path}"), WINDOW_WAIT)?;
    let place = installer_window(m, from)?;
    std::thread::sleep(Duration::from_secs(2));
    m.shot("installer-from-xdg-open")?;
    // What xdg-open started only asked: the window is the desktop's.
    let said = m.run("cat /tmp/xdg-open.log")?;
    expect(
        &format!("the installer xdg-open started handed the file to the desktop ({said:?})"),
        said.contains(&format!("poly-installer: {path}: handed to the desktop")),
    )?;
    let (x, y) = other_button(place);
    m.click(x, y)?;
    m.seen(from, &format!("poly-installer: {PACKAGE}: closed (Ready)"), WINDOW_WAIT)?;

    // 2. A double click on the file in Files.
    let from = m.text_len();
    m.send_line(
        ". /etc/poly-desktop.env; POLY_COMPOSITOR_CONTROL=$XDG_RUNTIME_DIR/poly-compositor.ctl \
         poly-files /home/poly/Downloads > /tmp/files.log 2>&1 &",
    )?;
    let rest = m.seen(from, "window \"Files\"", WINDOW_WAIT)?;
    let files = window_place(&rest).ok_or_else(|| format!("unreadable window line {rest:?}"))?;
    std::thread::sleep(Duration::from_secs(3));
    m.shot("files-downloads")?;
    let listing = m.run("cat /tmp/files.log")?;
    expect(&format!("Files lists the package in Downloads ({listing:?})"), listing.contains(deb))?;
    // The launcher came back when the installer's window went: away with
    // it, a click to select the file, then the double click.
    m.key("esc")?;
    let from = m.text_len();
    let (x, y) = first_row(files);
    m.click(x, y)?;
    std::thread::sleep(Duration::from_secs(1));
    m.double_click(x, y)?;
    m.seen(from, &format!("poly-compositor: asked to open the installer for {path}"), WINDOW_WAIT)?;
    let place = installer_window(m, from)?;
    std::thread::sleep(Duration::from_secs(2));
    m.shot("installer-ready")?;

    // 3. Install: the base root is configured, the lists fetched, the
    // package installed with what it needs.
    let from = m.text_len();
    let (x, y) = main_button(place);
    m.click(x, y)?;
    m.seen(from, "poly-installer: running /usr/bin/dpkg", WINDOW_WAIT)?;
    std::thread::sleep(Duration::from_secs(8));
    m.shot("installer-preparing")?;
    const ENDS: &str = "poly-installer: failed: ";
    let step = |m: &mut Machine, what: &str| -> Result {
        // A failure ends the wait with what the installer said.
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
            std::thread::sleep(Duration::from_millis(200));
        }
    };
    step(m, "poly-installer: running /usr/bin/apt-get update")?;
    step(m, "poly-installer: running /usr/bin/apt-get install")?;
    std::thread::sleep(Duration::from_millis(1500));
    m.shot("installer-installing")?;
    step(m, &format!("poly-installer: installed {PACKAGE} "))?;
    let done = m.seen(from, &format!("poly-installer: installed {PACKAGE} "), WINDOW_WAIT)?;
    expect(
        &format!("the installer names the launcher entry the package brought ({done:?})"),
        done.contains(&format!("{APP} ({PACKAGE}.desktop)")),
    )?;
    // 4. The desktop read its apps again.
    m.seen(from, "app(s) in the launcher (read again)", WINDOW_WAIT)?;
    std::thread::sleep(Duration::from_secs(2));
    m.shot("installer-done")?;
    let out = m.run(&format!(
        "echo dpkg-\"$(dpkg -s {PACKAGE} | grep -c '^Status: install ok installed')-$(dpkg -s libquadmath0 | grep -c '^Status: install ok installed')-\
         $(dpkg -l | grep -c '^i[^i]')-$(dpkg --audit | wc -l)-$(ls /usr/share/applications/{PACKAGE}.desktop | wc -l)\""
    ))?;
    expect(
        &format!("dpkg has the app and its dependency installed, and nothing is left unconfigured ({out:?})"),
        out.contains("dpkg-1-1-0-0-1"),
    )?;

    // 5. Open, from the installer: the desktop starts the app.
    let from = m.text_len();
    let (x, y) = main_button(place);
    m.click(x, y)?;
    m.seen(from, &format!("poly-compositor: asked to open {APP} ({PACKAGE}.desktop)"), WINDOW_WAIT)?;
    m.seen(from, &format!("window \"{PACKAGE}\""), WINDOW_WAIT)?;
    std::thread::sleep(Duration::from_secs(3));
    m.shot("app-running")?;
    let out = m.run(&format!("echo runs-\"$(pidof {PACKAGE} | wc -w)\""))?;
    expect(&format!("the app runs ({out:?})"), out.contains("runs-1"))?;

    // The launcher finds it, with no restart of the desktop, and starts it.
    let from = m.text_len();
    m.key("meta_l")?;
    m.seen(from, "launcher shown", WINDOW_WAIT)?;
    m.type_text("galc")?;
    m.seen(from, "launcher query \"galc\"", WINDOW_WAIT)?;
    std::thread::sleep(Duration::from_secs(2));
    m.shot("launcher-finds-the-app")?;
    // The open window is listed first (Alpine lists windows with the apps);
    // the app is the entry below it.
    m.key("down")?;
    m.key("ret")?;
    m.seen(from, &format!("poly-compositor: launched {APP}"), WINDOW_WAIT)?;
    let how = m.seen(from, &format!("poly-compositor: launch {APP}: "), WINDOW_WAIT)?;
    expect(
        &format!("the app started from the launcher shows a window ({how:?})"),
        how.starts_with("window mapped") || how.starts_with("activated"),
    )?;

    // Files' Applications folder lists it (ADR 0175): the same catalog.
    let from = m.text_len();
    m.send_line(
        ". /etc/poly-desktop.env; POLY_COMPOSITOR_CONTROL=$XDG_RUNTIME_DIR/poly-compositor.ctl \
         poly-files applications: > /tmp/files-apps.log 2>&1 &",
    )?;
    m.seen(from, "window \"Files\"", WINDOW_WAIT)?;
    std::thread::sleep(Duration::from_secs(3));
    m.shot("files-applications")?;
    let listing = m.run("cat /tmp/files-apps.log")?;
    expect(
        &format!("Files' Applications lists the app ({listing:?})"),
        listing.lines().any(|l| l.contains("applications: (") && l.split(' ').any(|n| n == APP)),
    )
}

/// The boot test.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    if !cfg!(unix) {
        eprintln!("==> skipped: the linux-installer boot test needs a Unix host (QMP)");
        return Ok(());
    }
    let locked = chromium::lock()?;
    let absent = chromium::missing(&locked, &chromium::BUILD_SETS);
    if !absent.is_empty() {
        eprintln!(
            "==> skipped: the linux-installer boot test: {} locked Debian packages not downloaded (cargo xtask chromium fetch)",
            absent.len()
        );
        return Ok(());
    }
    let package = locked
        .iter()
        .find(|p| p.set == "install" && p.name == PACKAGE)
        .ok_or_else(|| format!("{PACKAGE} is not in {} (set install)", chromium::LOCK))?;
    let deb = package.file_name().to_string();
    // Chromium's disk with the package in the Downloads folder.
    let files = PolyFiles::default();
    let mut contents = chromium::collect(&files)?;
    contents.tree.dir("home/poly/Downloads", 0o755);
    contents.tree.host_file(&format!("home/poly/Downloads/{deb}"), 0o644, &chromium::debs_dir().join(&deb), 0);
    let image = root().join("target").join(IMAGE);
    rootdisk::write_image_poly_ext2(&contents.tree, &image)?;
    let initrd = chromium::write_initramfs(&files)?;
    eprintln!(
        "==> boot test: {deb} opened, installed by Poly's installer with apt and dpkg, found in the launcher, started"
    );
    let init = format!("{} poly.net=qemu", chromium::INIT_ARGS);
    let mut m = Machine::boot_with_init_args(
        kernel,
        syms,
        &image,
        &initrd,
        "linux-installer-test",
        chromium::MEMORY,
        &init,
        chromium::CPUS,
        &crate::net::nic_args(None, false),
    )?;
    m.set_timeouts(Duration::from_secs(600), Duration::from_secs(3600));
    m.desktop_up()?;
    let result = scenario(&mut m, &deb);
    let log = root().join("target/poly-linux-installer-boot.log");
    let _ = std::fs::write(&log, m.text());
    let _ = m.poweroff();
    result.map_err(|e| {
        eprintln!("==> the console's output is in {}", log.display());
        format!("installing a Linux app: {e}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_buttons_are_found_from_the_windows_place() {
        // The installer's window on a 1280x800 desktop, as the compositor
        // reports it.
        let place = window_place(" shown at 380,224 size 520x400").unwrap();
        assert_eq!(main_button(place), (826, 586));
        assert_eq!(other_button(place), (724, 586));
        assert_eq!(first_row((100, 50, 900, 580)), (464, 195));
    }

    #[test]
    fn the_test_installs_a_package_the_lock_pins_and_the_disk_lacks() {
        let locked = chromium::lock().unwrap();
        let install: Vec<&str> = locked.iter().filter(|p| p.set == "install").map(|p| p.name.as_str()).collect();
        assert_eq!(install, chromium::INSTALL_SEEDS);
        assert!(install.contains(&PACKAGE));
        // Its dependency that is not on the disk either: apt has to fetch it.
        assert!(!locked.iter().any(|p| p.name == "libquadmath0" && p.set == "disk"));
        assert!(locked.iter().any(|p| p.name == "apt" && p.set == "disk"));
        assert!(locked.iter().any(|p| p.name == "dpkg" && p.set == "disk"));
    }
}
