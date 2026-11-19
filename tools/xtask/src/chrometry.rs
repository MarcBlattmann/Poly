//! Chrome for Windows on Poly's desktop, to try by hand (ADR 0216): the menu entry "Desktop with
//! Chrome for Windows" of `cargo xtask start`, or `cargo xtask desktop --chrome-win`.
//!
//! Until now Chrome ran only in scripted checks. This is the same Chrome for Testing
//! ([`crate::chromewin::VERSION`], `cargo xtask chrome-win fetch` downloads it) as an installed
//! program: a disk with Chrome under `C:\Program Files\Chrome`, and a launcher entry named
//! [`ENTRY_NAME`] in Poly's desktop, which boots in QEMU's window with user networking.
//!
//! - **The disk** ([`disk`]) is `target/poly-chrome-try-disk.img`: Chrome, `polystub.dll` and
//!   Poly's fonts (`chromewin::app_tree`), none of the test files. Chrome's profile lives on it
//!   ([`PROFILE_DOS`], on `C:`), so bookmarks, history and logins survive a reboot; the disk is made
//!   again only when Chrome's version, `polystub.dll` or the fonts change (or with `--fresh-disk`),
//!   and then the profile starts empty.
//! - **The launcher entry** ([`desktop_entry`]) is a desktop file in the initramfs, so it follows the
//!   checkout while the disk is kept. Chrome's switches are in [`FLAGS`], the one place to change
//!   them: the window check ([`group_boot_test`]) and the stage 3 check use the same list.
//! - **Mounts**: the disk is `virtio0`, mounted on [`MOUNT`]; the settings disk of the desktop
//!   (layout, scale, shortcuts), when there is one, is `virtio1` on `/mnt` ([`mounts`]).
//!
//! Everything here builds on a Windows and a Linux host (ADR 0124): the disk is written by the
//! repository's own formatter. Only [`group_boot_test`] needs Linux (QMP over a Unix socket).

use crate::Result;
use std::path::{Path, PathBuf};

/// Chrome's switches in the launcher's entry. **The one place**, and empty: Chrome starts with its
/// defaults, as on Windows, its sandbox on (S2, ADR 0206/0217) and its GPU on Poly's software
/// Direct3D 11 device (S5, ADR 0211), so it shows no "unsupported command-line flag" bar (ADR 0216
/// addendum). The window check ([`group_boot_test`]) and stage 3 use the same list.
pub const FLAGS: &[&str] = &[];
/// Not about what Poly lacks: a first start of a browser (no welcome page, no question about
/// being the default browser) and where the profile is ([`PROFILE_DOS`]).
const START_FLAGS: &[&str] = &["--no-first-run", "--no-default-browser-check"];
/// Chrome's profile, on the disk's `C:`.
pub const PROFILE_DOS: &str = "C:\\poly-chrome-profile";

/// The disk image.
pub const DISK_FILE: &str = "poly-chrome-try-disk.img";
/// The initramfs (the desktop with the launcher entry).
const INITRAMFS_FILE: &str = "poly-chrome-try-initramfs.cpio";
/// Guest memory: Chrome needs about 4 GB.
pub const MEMORY: &str = "4G";
pub const CPUS: u32 = 4;
/// Where the kernel mounts the disk in the guest.
pub const MOUNT: &str = "/winc";

/// The entry's name in the launcher, and what a check types to find it.
pub const ENTRY_NAME: &str = "Chrome (Windows)";
/// The desktop file's path below the initramfs's root.
const ENTRY_PATH: &str = "usr/share/applications/chrome-windows.desktop";
/// The icon's path (an absolute path as `Icon=`, ADR 0146).
const ICON_PATH: &str = "usr/share/pixmaps/chrome-windows.png";
const ICON_IN_GUEST: &str = "/usr/share/pixmaps/chrome-windows.png";

/// The `poly.mount=` value: the disk (`virtio0`) on [`MOUNT`], and with `settings` the desktop's
/// settings disk (`virtio1`) on `/mnt`, where its `poly/shell.conf` is.
pub fn mounts(settings: bool) -> String {
    if settings { format!("virtio0:{MOUNT},virtio1:/mnt") } else { format!("virtio0:{MOUNT}") }
}

/// Chrome's command line after the program: [`FLAGS`], the start switches and the profile.
pub fn switches() -> Vec<String> {
    FLAGS.iter().chain(START_FLAGS).map(|s| s.to_string()).chain([format!("--user-data-dir={PROFILE_DOS}")]).collect()
}

/// Whether [`FLAGS`] turn Chrome's GPU off (then there is no GPU process to check).
fn gpu_off() -> bool {
    FLAGS.contains(&"--disable-gpu")
}

/// Where Chrome's program is, as the launcher's `Exec` names it.
fn exe_in_guest() -> String {
    format!("{MOUNT}/{}/chrome.exe", crate::chromewin::APP_ON_DISK)
}

/// The launcher entry's desktop file; `more` is added after the switches (a page, a window's place:
/// the window check's). `%` is `%%` in `Exec` (Desktop Entry Specification, "The Exec key").
pub fn desktop_entry(more: &[&str], icon: bool) -> String {
    let args: Vec<String> = switches().into_iter().chain(more.iter().map(|s| s.to_string())).collect();
    let exec = format!("\"{}\" {}", exe_in_guest(), args.join(" ").replace('%', "%%"));
    let icon = if icon { format!("Icon={ICON_IN_GUEST}\n") } else { String::new() };
    format!(
        "[Desktop Entry]\nType=Application\nName={ENTRY_NAME}\nGenericName=Web Browser\n\
         Comment=Chrome for Testing {} (a Windows program) running on Poly\n{icon}Exec={exec}\n\
         Keywords=chrome;google;browser;web;internet;windows;\n",
        crate::chromewin::VERSION
    )
}

/// The initramfs's files for the entry: its desktop file and, when Chrome's own icon can be read
/// from its program, the icon.
pub fn entry_files(more: &[&str]) -> Vec<(String, u16, Vec<u8>)> {
    let png = icon_png();
    let mut files = vec![(ENTRY_PATH.to_string(), 0o644, desktop_entry(more, png.is_some()).into_bytes())];
    if let Some(png) = png {
        files.push((ICON_PATH.to_string(), 0o644, png));
    }
    files
}

/// Chrome's own icon: the largest PNG image of the first icon group in `chrome.exe`'s resources
/// (names and pictures only; nothing of its code is read). `None` when there is none: the shell
/// then draws its own glyph.
pub fn icon_png() -> Option<Vec<u8>> {
    let bytes = std::fs::read(crate::chromewin::app_dir().join("chrome.exe")).ok()?;
    icon_png_of(&bytes)
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// [`icon_png`] of a program's bytes.
fn icon_png_of(exe: &[u8]) -> Option<Vec<u8>> {
    use poly_pe::ResourceKey;
    let pe = poly_pe::Pe::parse(exe).ok()?;
    let root = pe.resources().ok()??;
    // RT_GROUP_ICON (14): the first group, its first language.
    let groups = root.find(ResourceKey::Id(14)).ok()??.directory().ok()??;
    let group = groups.entries().next()?.ok()?.directory().ok()??;
    let data = group.entries().next()?.ok()?.data().ok()??;
    let dir = pe.resource_bytes(&data).ok()?;
    // GRPICONDIR: reserved, type, count; then 14-byte entries: width, height, colours, reserved,
    // planes, bits, size of the image, the RT_ICON's id.
    let count = usize::from(u16::from_le_bytes([*dir.get(4)?, *dir.get(5)?]));
    let mut best: Option<(u32, Vec<u8>)> = None;
    let icons = root.find(ResourceKey::Id(3)).ok()??.directory().ok()??;
    for i in 0..count {
        let e = dir.get(6 + i * 14..6 + i * 14 + 14)?;
        let width = if e[0] == 0 { 256 } else { u32::from(e[0]) };
        let id = u32::from(u16::from_le_bytes([e[12], e[13]]));
        let image = icons.find(ResourceKey::Id(id)).ok()??.directory().ok()??.entries().next()?.ok()?.data().ok()??;
        let image = pe.resource_bytes(&image).ok()?;
        if image.starts_with(PNG_SIGNATURE) && best.as_ref().is_none_or(|(w, _)| width > *w) {
            best = Some((width, image.to_vec()));
        }
    }
    best.map(|(_, png)| png)
}

/// The disk: Chrome as an installed program and what it needs of Poly's own, made again when
/// Chrome's version, `polystub.dll` or the fonts changed (or with `fresh`: the profile is lost).
pub fn disk(fresh: bool) -> Result<PathBuf> {
    let image = crate::root().join("target").join(DISK_FILE);
    let stamp = image.with_extension("stamp");
    let stubs = crate::chromewin::polystub()?;
    let print = crate::chromewin::fingerprint(&[
        crate::chromewin::VERSION.as_bytes(),
        &stubs,
        crate::chromewin::FONTS_STAMP.as_bytes(),
        crate::chromewin::APP_ON_DISK.as_bytes(),
    ]);
    if !fresh && image.exists() && std::fs::read_to_string(&stamp).is_ok_and(|s| s.trim() == print) {
        crate::progress::note(&format!(
            "Chrome's disk {} is kept from before (--fresh-disk starts again)",
            image.display()
        ));
        return Ok(image);
    }
    let started = std::time::Instant::now();
    let tree = crate::chromewin::app_tree(&stubs, None)?;
    crate::rootdisk::write_image_poly_ext2(&tree, &image)?;
    std::fs::write(&stamp, &print).map_err(|e| format!("{}: {e}", stamp.display()))?;
    crate::progress::note(&format!(
        "Chrome's disk {} ({} entries, {:.0?})",
        image.display(),
        tree.entries.len(),
        started.elapsed()
    ));
    Ok(image)
}

/// What the desktop boots from: Chrome's disk and the desktop's initramfs with the launcher entry.
#[derive(Debug)]
pub struct Parts {
    pub initrd: PathBuf,
    pub disk: PathBuf,
}

/// Fetch Chrome when it is missing, make the disk and the initramfs (`desktop --chrome-win`).
pub fn parts(fresh_disk: bool) -> Result<Parts> {
    if !crate::chromewin::fetched() {
        crate::progress::note(&format!("fetching Chrome for Testing {} (210 MB, once)", crate::chromewin::VERSION));
        crate::chromewin::fetch()?;
    }
    let disk = disk(fresh_disk)?;
    let initrd = crate::desktop::image_with(INITRAMFS_FILE, false, entry_files(&[]))?;
    Ok(Parts { initrd, disk })
}

// ---- the boot check -------------------------------------------------------------

/// The steps of stage 3 (`chromewin::S3_STEPS`) with a screenshot after each key step, so a failed
/// check shows where the typed address went.
const TRY_STEPS: &str = concat!(
    "launch,sleep:50,click:143|103,sleep:3,click:143|103,sleep:70,pixel:200|400|008000,",
    "chord:ctrl+l,sleep:2,shot:address-focused,type:data:text/html,<body bgcolor=blue>,",
    "shot:typed,key:ret,sleep:10,shot:navigated,pixel:200|400|0000ff,chord:alt+f,sleep:8,shot:menu"
);

/// The window check of what the menu boots: the same disk and entry as `desktop --chrome-win`
/// (the entry only adds the first page and where the window is), started from the launcher by its
/// name, then the page of stage 3 is painted after 125 s, a URL typed into the address field loads
/// (`chromewin::S3_STEPS`); the boot test group `chrome-try`, in the shard of `chrome-win-window`. Skipped where Chrome is not fetched or there is no Unix host (QMP
/// over a Unix socket); an exception, a window that never came or a wrong colour fails it, and a
/// page that never finished loading (the slow start) is inconclusive after three runs, as stage 3.
pub fn group_boot_test(kernel: &Path, _syms: &std::sync::Arc<crate::symbols::Symbols>) -> Result {
    eprintln!("==> boot test: Chrome for Windows from the menu's disk and launcher entry ({ENTRY_NAME}, ADR 0216)");
    if !crate::chromewin::fetched() {
        eprintln!("==> skipped: Chrome is not in the cache; `cargo xtask chrome-win fetch` downloads it");
        return Ok(());
    }
    if !cfg!(unix) {
        eprintln!("==> skipped: a desktop run needs a Unix host (QMP over a Unix socket)");
        return Ok(());
    }
    let image = disk(false)?;
    let log = crate::root().join("target/win-app/try-boot/chrome-try.serial.log");
    let more = ["--window-position=40,40", "--window-size=900,500", crate::chromewin::S3_PAGE];
    for attempt in 1..=3 {
        let (text, stopped) = crate::winapps::desktop_entry_check(
            kernel,
            &image,
            entry_files(&more),
            &mounts(false),
            ENTRY_NAME,
            TRY_STEPS,
            &log,
        )?;
        let exceptions: Vec<&str> = text.lines().filter(|l| l.contains("nt-exc:")).take(3).collect();
        match crate::chromewin::s3_verdict(stopped.as_deref(), &exceptions) {
            crate::chromewin::S3Verdict::Pass => {
                // With Chrome's defaults the page must come from one GPU process in its sandbox.
                if !gpu_off()
                    && let Some(why) = crate::chromewin::defaults_gpu_note(&text)
                {
                    return Err(format!("the menu's Chrome painted, but {why} (see {})", log.display()));
                }
                println!("CHROME TRY PASSED");
                return Ok(());
            }
            crate::chromewin::S3Verdict::Inconclusive => {
                eprintln!("==> attempt {attempt}: the page was still loading (slow start); log {}", log.display());
            }
            // A typed address that did not load (the page is still the green one): under a busy
            // machine the browser's first keys can be lost, as the slow start of stage 3; another
            // run, and a failure only when every run shows it.
            crate::chromewin::S3Verdict::Fail
                if attempt < 3
                    && stopped.as_deref().is_some_and(|w| w.contains("is [00, 80, 00], wanted [00, 00, ff]")) =>
            {
                eprintln!("==> attempt {attempt}: the typed address did not load; log {}", log.display());
            }
            crate::chromewin::S3Verdict::Fail => {
                return Err(format!(
                    "the menu's Chrome did not pass: {}; exceptions: {exceptions:?} (see {})",
                    stopped.unwrap_or_else(|| "the steps ran".into()),
                    log.display()
                ));
            }
        }
    }
    println!("CHROME TRY INCONCLUSIVE: the page never finished loading in 3 runs (slow start); no exception either");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(more: &[&str]) -> poly_shell::apps::App {
        poly_shell::apps::parse_desktop_entry("chrome-windows.desktop", &desktop_entry(more, true)).unwrap()
    }

    #[test]
    fn the_launcher_lists_chrome_under_its_name_and_icon() {
        let app = entry(&[]);
        assert_eq!(app.name, ENTRY_NAME);
        assert_eq!(app.icon.as_deref(), Some(ICON_IN_GUEST));
        assert!(app.keywords.iter().any(|k| k == "browser"));
    }

    #[test]
    fn it_starts_chrome_from_the_disk_with_the_flags_of_one_place() {
        let app = entry(&[]);
        assert_eq!(app.argv[0], "/winc/poly/nt/c/Program Files/Chrome/chrome.exe");
        let flags: Vec<&str> = FLAGS.to_vec();
        assert_eq!(app.argv[1..1 + flags.len()], flags[..]);
        // The profile is on the disk's C:, with the backslash as it is.
        assert!(app.argv.contains(&format!("--user-data-dir={PROFILE_DOS}")), "{:?}", app.argv);
    }

    #[test]
    fn chrome_starts_with_its_defaults_sandbox_and_gpu_on() {
        // No switch that turns off a security or GPU feature: Chrome shows its "unsupported
        // command-line flag" bar for those (ADR 0216 addendum).
        for f in switches() {
            assert!(
                ![
                    "--no-sandbox",
                    "--disable-gpu",
                    "--disable-gpu-sandbox",
                    "--disable-gpu-compositing",
                    "--in-process-gpu"
                ]
                .iter()
                .any(|bad| f == *bad),
                "{f}"
            );
        }
        assert!(!gpu_off());
    }

    #[test]
    fn a_page_and_a_percent_reach_chrome_as_given() {
        let app = entry(&["--window-size=900,500", "data:text/html,<b>a%20b</b>"]);
        assert_eq!(app.argv.last().unwrap(), "data:text/html,<b>a%20b</b>");
        assert!(app.argv.iter().any(|a| a == "--window-size=900,500"));
        // The window check's page is one argument too.
        let app = entry(&[crate::chromewin::S3_PAGE]);
        assert_eq!(app.argv.last().unwrap(), crate::chromewin::S3_PAGE);
    }

    #[test]
    fn the_disk_is_the_first_mount_and_the_settings_the_second() {
        assert_eq!(mounts(false), "virtio0:/winc");
        assert_eq!(mounts(true), "virtio0:/winc,virtio1:/mnt");
    }

    #[test]
    fn the_entry_is_in_the_initramfs_without_an_icon_when_chrome_has_none() {
        let files = entry_files(&[]);
        assert_eq!(files[0].0, ENTRY_PATH);
        assert!(files.len() <= 2);
        assert!(String::from_utf8(files[0].2.clone()).unwrap().contains("Name=Chrome (Windows)"));
        assert_eq!(icon_png_of(b"not a program"), None);
    }

    #[test]
    fn chromes_own_icon_is_a_png_where_chrome_is_fetched() {
        if !crate::chromewin::fetched() {
            return;
        }
        let png = icon_png().expect("chrome.exe has an icon group with a PNG image");
        assert!(png.starts_with(PNG_SIGNATURE));
        // The width is in the IHDR chunk: at least the 48 pixels the launcher draws.
        assert!(u32::from_be_bytes([png[16], png[17], png[18], png[19]]) >= 48);
    }
}
