//! Boot group `linux-installer-2`: uninstalling a Linux app and installing an
//! AppImage, the way a person does it (roadmap Phase 8, ADR 0180; the
//! `.deb` half is ADR 0173, `installer.rs`).
//!
//! The desktop boots from Chromium's disk (`chromium.rs`) with **galculator**'s
//! `.deb` in the Downloads folder, as a download leaves it:
//!
//! 1. The `.deb` is opened as Chromium opens it (`xdg-open`, as user `poly`)
//!    and installed with Debian's `apt`; the app and the library `apt` had to
//!    fetch for it are on the disk.
//! 2. **Uninstall:** in Files' Applications folder a right click on Galculator
//!    offers *Uninstall*; the installer's window says what goes and asks;
//!    Debian's own `apt-get remove` and `autoremove` take the app and the
//!    library away; the launcher entry and the Files entry are gone and dpkg
//!    has neither package. Chromium, a package of the base root, is refused.
//! 3. **AppImage:** `curl` is installed from Debian's archive and downloads
//!    CPU-X's AppImage from the project's release page in the guest (pinned
//!    by URL and SHA-256). A double click in Files asks *Run* or *Install*;
//!    Install copies it to the user's folder, takes its desktop file and
//!    icon out of the image, and the launcher and Applications list it; Open
//!    starts it (a window appears); Uninstall from Applications removes it.
//!
//! The guest reaches Debian's snapshot archive and GitHub through QEMU's user
//! networking. The group skips itself where the packages are not fetched
//! (`cargo xtask chromium fetch`) and on hosts without Unix sockets (QMP),
//! so it does not run in CI, as `linux-installer` does not.

use crate::chromium::{self, PolyFiles};
use crate::installer::{STEP_WAIT, WINDOW_WAIT, expect, first_row, main_button, other_button};
use crate::symbols::Symbols;
use crate::toolkits::{Machine, window_place};
use crate::{Result, root, rootdisk};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The image the test makes afresh.
const IMAGE: &str = "poly-installer2-test.img";

const PACKAGE: &str = "galculator";
const APP: &str = "Galculator";
const DEB_ID: &str = "galculator.desktop";
/// Chromium's launcher entry on this disk (Poly's own; Debian's is not on it).
const CHROMIUM_ID: &str = chromium::DESKTOP_ID;

/// The proof AppImage: CPU-X 5.4.1 (GTK 3, GPL-3.0-or-later), from the
/// project's release page; the SHA-256 is the one the release lists.
const APPIMAGE_URL: &str =
    "https://github.com/TheTumultuousUnicornOfDarkness/CPU-X/releases/download/v5.4.1/CPU-X-5.4.1-x86_64.AppImage";
const APPIMAGE_SHA256: &str = "a05e4a7e3bd62afb900c9d3b5d6db8abb3e692e817465730773ed831d02118b2";
const APPIMAGE_FILE: &str = "CPU-X-5.4.1-x86_64.AppImage";
const APPIMAGE_APP: &str = "CPU-X";
const APPIMAGE_ID: &str = "poly-appimage-cpu-x.desktop";
/// Where the installer puts it (root's data directory, the desktop's user).
const APPIMAGE_DIR: &str = "/root/.local/share/poly/appimages/cpu-x";

/// A right click's menu: its rows are 28 high below a 5 pixel margin and the
/// menu starts at the click, so row `n` (from 0) is at this offset.
fn menu_row(n: i32) -> (i32, i32) {
    (20, 5 + 28 * n + 14)
}

/// The apps Files' Applications folder lists, in its order, from its log line
/// (`poly-files: applications: (N items): A B C`); `None` if a name has a
/// space in it (the row cannot be told from the words).
fn listing(log: &str) -> Option<Vec<String>> {
    let line = log.lines().rev().find(|l| l.contains("poly-files: applications: ("))?;
    let (head, names) = line.split_once("items): ")?;
    let count: usize = head.rsplit('(').next()?.trim().parse().ok()?;
    let names: Vec<String> = names.split_whitespace().map(String::from).collect();
    (names.len() == count).then_some(names)
}

/// A window's place: x, y, width, height.
type Place = (i32, i32, i32, i32);

/// The place of the window titled `title`, as the compositor reports it.
fn window(m: &mut Machine, from: usize, title: &str) -> Result<Place> {
    let rest = m.seen(from, &format!("window \"{title}\""), WINDOW_WAIT)?;
    window_place(&rest).ok_or_else(|| format!("unreadable window line {rest:?}"))
}

/// Wait for `what` in the console after `from`; a failure of the installer
/// ends the wait with what it said.
fn wait_for(m: &mut Machine, from: usize, what: &str) -> Result {
    const ENDS: &str = "poly-installer: failed: ";
    let deadline = Instant::now() + STEP_WAIT;
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
        if Instant::now() > deadline {
            return Err(format!("no {what:?} within {} s", STEP_WAIT.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Files' Applications folder in a window of its own: the window's place and
/// the apps it lists.
fn applications(m: &mut Machine, log: &str) -> Result<(Place, Vec<String>)> {
    let from = m.text_len();
    m.send_line(&format!(
        ". /etc/poly-desktop.env; POLY_COMPOSITOR_CONTROL=$XDG_RUNTIME_DIR/poly-compositor.ctl \
         poly-files applications: > {log} 2>&1 &"
    ))?;
    let files = window(m, from, "Files")?;
    std::thread::sleep(Duration::from_secs(3));
    let text = m.run(&format!("cat {log}"))?;
    let names = listing(&text).ok_or_else(|| format!("Files' Applications is not a list of single words: {text:?}"))?;
    Ok((files, names))
}

/// Right click on the app listed `name` in Files' Applications (the window
/// at `files`, the list as `names` has it) and click *Uninstall*, the menu's
/// second row.
fn uninstall_from_menu(m: &mut Machine, files: Place, names: &[String], name: &str) -> Result {
    let at = names.iter().position(|n| n == name).ok_or_else(|| format!("{name} is not in {names:?}"))?;
    let (x, y) = first_row(files);
    let y = y + 34 * at as i32;
    // The launcher is shown on an empty desktop and has the keyboard.
    m.key("esc")?;
    m.right_click(x, y)?;
    std::thread::sleep(Duration::from_secs(1));
    m.shot(&format!("menu-of-{}", name.to_lowercase()))?;
    let (dx, dy) = menu_row(1);
    m.click(x + dx, y + dy)
}

fn scenario(m: &mut Machine, deb: &str) -> Result {
    let path = format!("/home/poly/Downloads/{deb}");
    m.key("esc")?;

    // 1. Install galculator as a download is installed.
    let from = m.text_len();
    m.send_line(&format!("/usr/local/bin/poly-as-user xdg-open {path} > /tmp/xdg-open.log 2>&1 &"))?;
    m.seen(from, &format!("poly-compositor: asked to open the installer for {path}"), WINDOW_WAIT)?;
    let place = window(m, from, &format!("Install {PACKAGE}"))?;
    std::thread::sleep(Duration::from_secs(2));
    let from = m.text_len();
    let (x, y) = main_button(place);
    m.click(x, y)?;
    wait_for(m, from, &format!("poly-installer: installed {PACKAGE} "))?;
    m.seen(from, "app(s) in the launcher (read again)", WINDOW_WAIT)?;
    let out = m.run(&format!(
        "echo dpkg-\"$(dpkg -s {PACKAGE} | grep -c '^Status: install ok installed')-$(dpkg -s libquadmath0 | grep -c '^Status: install ok installed')-$(dpkg -s chromium | grep -c '^Status: install ok')\""
    ))?;
    expect(&format!("galculator and its library are installed ({out:?})"), out.contains("dpkg-1-1-1"))?;
    // The installer's window is open with Open and Close: Close.
    let from = m.text_len();
    let (x, y) = other_button(place);
    m.click(x, y)?;
    m.seen(from, &format!("poly-installer: {PACKAGE}: closed (Done)"), WINDOW_WAIT)?;

    // 2. Uninstall it from Files' Applications folder.
    let (files, names) = applications(m, "/tmp/files-apps-1.log")?;
    expect(
        &format!("Applications lists {APP} and Chromium ({names:?})"),
        names.iter().any(|n| n == APP) && names.iter().any(|n| n == "Chromium"),
    )?;
    m.shot("files-applications-with-the-app")?;
    let from = m.text_len();
    uninstall_from_menu(m, files, &names, APP)?;
    m.seen(from, &format!("poly-compositor: asked to uninstall {DEB_ID}"), WINDOW_WAIT)?;
    let place = window(m, from, &format!("Uninstall {APP}"))?;
    std::thread::sleep(Duration::from_secs(2));
    m.shot("uninstall-asks")?;
    // Nothing is removed before the click.
    let out = m.run(&format!("echo still-\"$(dpkg -s {PACKAGE} | grep -c '^Status: install ok installed')\""))?;
    expect(&format!("the window only asks ({out:?})"), out.contains("still-1"))?;
    let from = m.text_len();
    let (x, y) = main_button(place);
    m.click(x, y)?;
    m.seen(from, "poly-installer: running /usr/bin/apt-get remove", WINDOW_WAIT)?;
    wait_for(m, from, &format!("poly-installer: uninstalled {APP}: "))?;
    let done = m.seen(from, &format!("poly-installer: uninstalled {APP}: "), WINDOW_WAIT)?;
    expect(&format!("the installer names the launcher entry that went ({done:?})"), done.contains(DEB_ID))?;
    m.seen(from, "app(s) in the launcher (read again)", WINDOW_WAIT)?;
    std::thread::sleep(Duration::from_secs(2));
    m.shot("uninstall-done")?;
    let out = m.run(&format!(
        "echo gone-\"$(dpkg -s {PACKAGE} 2>&1 | grep -c '^Status: install ok installed')-$(dpkg -s libquadmath0 2>&1 | grep -c '^Status: install ok installed')-\
         $(ls /usr/share/applications/{DEB_ID} 2>&1 | grep -c 'No such')-$(ls /usr/bin/{PACKAGE} 2>&1 | grep -c 'No such')-\
         $(dpkg -s chromium | grep -c '^Status: install ok installed')-$(dpkg --audit | wc -l)\""
    ))?;
    expect(
        &format!("the app and its library are gone, Chromium and dpkg's state are fine ({out:?})"),
        out.contains("gone-0-0-1-1-1-0"),
    )?;
    // Close the installer; Files' list no longer has it.
    let from = m.text_len();
    let (x, y) = main_button(place);
    m.click(x, y)?;
    m.seen(from, &format!("poly-installer: {APP}: closed (Done)"), WINDOW_WAIT)?;
    let (_, names) = applications(m, "/tmp/files-apps-2.log")?;
    expect(&format!("Applications no longer lists {APP} ({names:?})"), !names.iter().any(|n| n == APP))?;
    expect(&format!("Applications still lists Chromium ({names:?})"), names.iter().any(|n| n == "Chromium"))?;
    m.key("esc")?;

    // Chromium comes with the disk, not from an installer: asked to uninstall,
    // the window says no.
    let from = m.text_len();
    m.send_line(&format!(
        ". /etc/poly-desktop.env; echo 'uninstall {CHROMIUM_ID}' > $XDG_RUNTIME_DIR/poly-compositor.ctl"
    ))?;
    let place = window(m, from, "Uninstall Chromium")?;
    std::thread::sleep(Duration::from_secs(2));
    m.shot("uninstall-refused")?;
    let said = m.seen(from, &format!("poly-installer: uninstall {CHROMIUM_ID}: "), WINDOW_WAIT)?;
    expect(&format!("the installer refuses the base root's browser ({said:?})"), said.contains("Refused("))?;
    let from = m.text_len();
    let (x, y) = main_button(place);
    m.click(x, y)?;
    m.seen(from, "poly-installer: Chromium: closed (Refused(", WINDOW_WAIT)?;
    let out = m.run("echo chromium-\"$(dpkg -s chromium | grep -c '^Status: install ok installed')\"")?;
    expect(&format!("Chromium is still installed ({out:?})"), out.contains("chromium-1"))?;

    appimage(m)
}

fn appimage(m: &mut Machine) -> Result {
    // 3. The AppImage, downloaded in the guest from its release page.
    m.run("rm -f /home/poly/Downloads/*.deb")?;
    let out = m.run(
        "DEBIAN_FRONTEND=noninteractive apt-get install -y -q curl > /tmp/curl-install.log 2>&1; echo curl-install-$?",
    )?;
    expect(&format!("curl is installed from Debian's archive ({out:?})"), out.contains("curl-install-0"))?;
    let out = m.run(&format!(
        "curl -fsSL -o /home/poly/Downloads/{APPIMAGE_FILE} {APPIMAGE_URL}; echo curl-rc-$?; \
         sha256sum /home/poly/Downloads/{APPIMAGE_FILE}"
    ))?;
    expect(&format!("the AppImage downloaded ({out:?})"), out.contains("curl-rc-0"))?;
    expect(&format!("it is the release the test pins ({out:?})"), out.contains(APPIMAGE_SHA256))?;
    let from = m.text_len();
    m.send_line(
        ". /etc/poly-desktop.env; POLY_COMPOSITOR_CONTROL=$XDG_RUNTIME_DIR/poly-compositor.ctl \
         poly-files /home/poly/Downloads > /tmp/files-dl.log 2>&1 &",
    )?;
    let files = window(m, from, "Files")?;
    std::thread::sleep(Duration::from_secs(3));
    let listing = m.run("cat /tmp/files-dl.log")?;
    expect(&format!("Files lists the AppImage ({listing:?})"), listing.contains(APPIMAGE_FILE))?;
    m.shot("files-downloads-with-the-appimage")?;
    m.key("esc")?;
    let from = m.text_len();
    let (x, y) = first_row(files);
    m.click(x, y)?;
    std::thread::sleep(Duration::from_secs(1));
    m.double_click(x, y)?;
    let place = window(m, from, &format!("Install {APPIMAGE_APP}"))?;
    std::thread::sleep(Duration::from_secs(2));
    m.shot("appimage-asks")?;

    // Install: the files are copied, the desktop file and the icon written.
    let from = m.text_len();
    let (x, y) = main_button(place);
    m.click(x, y)?;
    wait_for(m, from, &format!("poly-installer: installed {APPIMAGE_APP} "))?;
    m.seen(from, "app(s) in the launcher (read again)", WINDOW_WAIT)?;
    std::thread::sleep(Duration::from_secs(2));
    m.shot("appimage-installed")?;
    let out = m.run(&format!(
        "echo placed-\"$(test -x {APPIMAGE_DIR}/cpu-x.AppImage && echo 1)-$(test -s {APPIMAGE_DIR}/icon.png && echo 1)-\
         $(grep -c '^X-Poly-AppImage=' /root/.local/share/applications/{APPIMAGE_ID})-$(cmp {APPIMAGE_DIR}/cpu-x.AppImage /home/poly/Downloads/{APPIMAGE_FILE} && echo same)\""
    ))?;
    expect(
        &format!("the image, its icon and its desktop file are in the user's folder ({out:?})"),
        out.contains("placed-1-1-1-same"),
    )?;

    // Open, from the installer: the desktop starts the app; a window appears.
    let from = m.text_len();
    let (x, y) = main_button(place);
    m.click(x, y)?;
    m.seen(from, &format!("poly-compositor: asked to open {APPIMAGE_APP} ({APPIMAGE_ID})"), WINDOW_WAIT)?;
    let how = m.seen(from, &format!("poly-compositor: launch {APPIMAGE_APP}: "), WINDOW_WAIT)?;
    expect(
        &format!("the AppImage shows a window ({how:?})"),
        how.starts_with("window mapped") || how.starts_with("activated"),
    )?;
    std::thread::sleep(Duration::from_secs(5));
    m.shot("appimage-running")?;
    let out = m.run("echo running-\"$(ps | grep -ci '[c]pu-x')\"")?;
    expect(&format!("the app runs ({out:?})"), !out.contains("running-0"))?;
    // Stop it before it is removed.
    m.run("for p in $(pidof cpu-x); do kill $p; done; sleep 2; echo stopped")?;

    // The launcher finds it, and Applications lists it.
    let (files, names) = applications(m, "/tmp/files-apps-3.log")?;
    expect(&format!("Applications lists {APPIMAGE_APP} ({names:?})"), names.iter().any(|n| n == APPIMAGE_APP))?;
    m.shot("files-applications-with-the-appimage")?;

    // Uninstall it from the menu.
    let from = m.text_len();
    uninstall_from_menu(m, files, &names, APPIMAGE_APP)?;
    m.seen(from, &format!("poly-compositor: asked to uninstall {APPIMAGE_ID}"), WINDOW_WAIT)?;
    let place = window(m, from, &format!("Uninstall {APPIMAGE_APP}"))?;
    std::thread::sleep(Duration::from_secs(2));
    m.shot("appimage-uninstall-asks")?;
    let from = m.text_len();
    let (x, y) = main_button(place);
    m.click(x, y)?;
    wait_for(m, from, &format!("poly-installer: uninstalled {APPIMAGE_APP}: "))?;
    m.seen(from, "app(s) in the launcher (read again)", WINDOW_WAIT)?;
    std::thread::sleep(Duration::from_secs(2));
    m.shot("appimage-uninstalled")?;
    let out = m.run(&format!(
        "echo removed-\"$(ls {APPIMAGE_DIR} 2>&1 | grep -c 'No such')-$(ls /root/.local/share/applications/{APPIMAGE_ID} 2>&1 | grep -c 'No such')-\
         $(test -f /home/poly/Downloads/{APPIMAGE_FILE} && echo kept)\""
    ))?;
    expect(&format!("its files are gone and the download is left ({out:?})"), out.contains("removed-1-1-kept"))?;
    Ok(())
}

/// The boot test.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    if !cfg!(unix) {
        eprintln!("==> skipped: the linux-installer-2 boot test needs a Unix host (QMP)");
        return Ok(());
    }
    let locked = chromium::lock()?;
    let absent = chromium::missing(&locked, &chromium::BUILD_SETS);
    if !absent.is_empty() {
        eprintln!(
            "==> skipped: the linux-installer-2 boot test: {} locked Debian packages not downloaded (cargo xtask chromium fetch)",
            absent.len()
        );
        return Ok(());
    }
    let package = locked
        .iter()
        .find(|p| p.set == "install" && p.name == PACKAGE)
        .ok_or_else(|| format!("{PACKAGE} is not in {} (set install)", chromium::LOCK))?;
    let deb = package.file_name().to_string();
    let files = PolyFiles::default();
    let mut contents = chromium::collect(&files)?;
    contents.tree.dir("home/poly/Downloads", 0o755);
    contents.tree.host_file(&format!("home/poly/Downloads/{deb}"), 0o644, &chromium::debs_dir().join(&deb), 0);
    let image = root().join("target").join(IMAGE);
    rootdisk::write_image_poly_ext2(&contents.tree, &image)?;
    let initrd = chromium::write_initramfs(&files)?;
    eprintln!("==> boot test: {deb} installed and uninstalled, an AppImage downloaded, installed, run and uninstalled");
    let init = format!("{} poly.net=qemu", chromium::INIT_ARGS);
    let mut m = Machine::boot_with_init_args(
        kernel,
        syms,
        &image,
        &initrd,
        "linux-installer-2-test",
        chromium::MEMORY,
        &init,
        chromium::CPUS,
        &crate::net::nic_args(None, false),
    )?;
    m.set_timeouts(Duration::from_secs(600), Duration::from_secs(3600));
    m.desktop_up()?;
    let result = scenario(&mut m, &deb);
    let log = root().join("target/poly-linux-installer-2-boot.log");
    let _ = std::fs::write(&log, m.text());
    let _ = m.poweroff();
    result.map_err(|e| {
        eprintln!("==> the console's output is in {}", log.display());
        format!("uninstalling a Linux app and installing an AppImage: {e}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_listing_is_read_from_files_log() {
        let log = "poly-files: 0 apps open files\npoly-files: applications: (6 items): Chromium Files Galculator Notifications Settings Terminal\n";
        let names = listing(log).unwrap();
        assert_eq!(names.iter().position(|n| n == "Galculator"), Some(2));
        // A name with a space is not told from two.
        assert_eq!(listing("poly-files: applications: (1 items): Hello Pixels\n"), None);
        assert_eq!(listing("nothing"), None);
        // The latest listing counts.
        let twice = format!("{log}poly-files: applications: (2 items): Files Terminal\n");
        assert_eq!(listing(&twice).unwrap(), ["Files", "Terminal"]);
    }

    #[test]
    fn menu_rows_and_the_pinned_download() {
        // Open, then Uninstall: 5 + 28 + 14 below the click.
        assert_eq!(menu_row(0), (20, 19));
        assert_eq!(menu_row(1), (20, 47));
        assert_eq!(APPIMAGE_SHA256.len(), 64);
        assert!(APPIMAGE_URL.starts_with("https://github.com/") && APPIMAGE_URL.ends_with(APPIMAGE_FILE));
        assert_eq!(poly_pkg::appimage::id_of(&poly_pkg::appimage::slug(APPIMAGE_APP)), APPIMAGE_ID);
    }
}
