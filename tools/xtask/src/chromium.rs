//! Chromium (the Linux build) on Poly's desktop with its sandbox enabled
//! (roadmap Phase 6, "Chromium (the Linux build) runs on the desktop with
//! its sandbox enabled"). What Chromium needs from Poly, as observed on
//! real Linux, is in `docs/chromium-linux-gaps.md`; its scenario tests are
//! `specs/linux/apps/chromium.tests/`.
//!
//! **Where Chromium comes from.** Debian 13 ("trixie")'s own `chromium`
//! packages, an open-source build (no Google Chrome binary is ever in a
//! Poly image), with exactly the Debian packages they depend on, from two
//! pinned snapshots of snapshot.debian.org ([`ARCHIVES`]: trixie at its
//! 13.7 point release and trixie-security, where Debian ships Chromium's
//! updates). Every package is pinned in `tools/xtask/chromium.lock`
//! (name, version, archive, size, SHA-256, set, file name): `cargo xtask
//! chromium lock` writes it from the snapshots' package indexes (each
//! checked against the SHA-256 its snapshot's `InRelease` gives) by
//! following `Depends` and `Pre-Depends` from [`DISK_SEEDS`],
//! [`BOOT_SEEDS`] and [`REFERENCE_SEEDS`] (the first alternative that
//! exists). Only the lock is committed: the packages are free software
//! Debian distributes, downloaded at build time into `target/chromium/debs/`.
//!
//! **Every host builds the disk** (the owner's requirement: a Windows build
//! gives the same desktop with Chromium in the launcher). All of it is
//! pure Rust: the download (`curl`) is checked with `sha256.rs`; a `.deb`
//! is an `ar(5)` archive whose `data.tar.xz` member is a tar archive
//! compressed with xz (`lzma-rs`; [`ar_members`], [`tar_entries`]); its
//! regular files are unpacked into numbered blobs under
//! `target/chromium/disk/` (no file names of the packages on the host file
//! system, so nothing depends on what names, links or modes the host
//! allows), and the tree is made from the archive entries: their paths,
//! modes (set-user-ID `chrome-sandbox` included), link targets and times.
//! The image is written by `poly-ext2`'s formatter on every host
//! ([`rootdisk::write_image_poly_ext2`]), so it is the same everywhere. No
//! maintainer scripts run; [`configure`] does what Chromium needs of them.
//!
//! - `cargo xtask chromium fetch`: download the locked packages, check them.
//! - `cargo xtask chromium disk`: make `target/poly-chromium-disk.img`: the
//!   packages of the `disk` set, the desktop as on the toolkits' disk
//!   (ADR 0120: the native compositor, the shell's fonts and wallpapers,
//!   `/etc/poly-desktop.env`), user `poly` and `poly-chromium`, which runs
//!   Chromium as that user (it will not run its sandbox as root), the
//!   launcher's desktop file, and the test pages ([`PAGES`]).
//! - `cargo xtask chromium run`: boot it with the desktop in QEMU's window
//!   and the disk's bash on the terminal (any host). The initramfs's
//!   BusyBox is Debian's `busybox-static` from the lock (set `boot`), so no
//!   host C toolchain is needed. The initramfs switches to the disk as the
//!   root (`initramfs::SWITCH_ROOT_SCRIPT`, ADR 0130's amendment): it is not
//!   a chroot, so Chromium may make user namespaces and takes its
//!   user-namespace sandbox; the same BusyBox, copied onto the disk
//!   (`initramfs::DISK_BUSYBOX`), is init there with [`INITTAB`].
//! - `cargo xtask chromium [--fresh-disk] [STEP...]`: a boot driven like
//!   `cargo xtask toolkits` (steps; QMP, so Unix hosts).
//! - `cargo xtask chromium apptest [--target linux|poly|both] [--tests-dir
//!   DIR]...`: the scenario tests (`specs/linux/apps/chromium.tests/*.app`,
//!   the pipeline's app test format with programs of Chromium's disk) on
//!   the reference and on Poly, stdout and exit status compared per test
//!   ([`AppTest`]).
//! - `cargo xtask chromium root --reference` (Linux, `dpkg-deb`): every
//!   locked package unpacked into `target/chromium/root-reference/`, the
//!   tree the spec agent observes Chromium in on real Linux (with weston,
//!   Xvfb, Xwayland, xdotool, xwd and strace, set `reference`).
//!
//! Boot group `chromium` ([`boot_test`]) is the milestone's test: headless
//! `--dump-dom` and `--screenshot` of the test pages from the console, then
//! Chromium windowed on the desktop with its sandbox on, clicked into. It
//! skips itself where the packages have not been fetched and on hosts
//! without Unix sockets (QMP).

use crate::rootdisk::{self, Contents, RootTree};
use crate::symbols::Symbols;
use crate::{Result, compositor, root};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

/// A pinned Debian archive snapshot: its package index (`Packages.xz`,
/// as listed in the snapshot's `InRelease`) and where its pool is.
pub struct Archive {
    /// The name the lock uses.
    pub name: &'static str,
    /// The snapshot's root (the pool's `Filename`s are relative to it).
    pub base: &'static str,
    /// The index, relative to `base`.
    pub index: &'static str,
    /// The index's size and SHA-256 (from the snapshot's `InRelease`).
    pub size: u64,
    pub sha256: &'static str,
}

/// The two snapshots. A package in both comes from the later one
/// (security updates, Chromium's among them).
pub const ARCHIVES: [Archive; 2] = [
    Archive {
        name: "debian",
        base: "https://snapshot.debian.org/archive/debian/20261031T202858Z",
        index: "dists/trixie/main/binary-amd64/Packages.xz",
        size: 9_678_380,
        sha256: "7778d3e3f303b7ddb8ce0fe7c8d57473a076c6bf2e8f241f75421d2396352498",
    },
    Archive {
        name: "debian-security",
        base: "https://snapshot.debian.org/archive/debian-security/20261031T222717Z",
        index: "dists/trixie-security/main/binary-amd64/Packages.xz",
        size: 291_652,
        sha256: "a05d49ec16890db9bdbbe25ecbcb9d106d06bbaf410b4420fb9f2575c777a7e6",
    },
];

/// What Poly's disk is made from: Chromium, its setuid sandbox helper
/// (`chrome-sandbox`, used where user namespaces are not available), and
/// what Debian takes for granted (Essential packages are nobody's
/// dependency): the shells Chromium's start script and the console need,
/// coreutils, findutils and grep (the start script uses them), and
/// `util-linux` for `setpriv` and `findmnt`.
pub const DISK_SEEDS: &[&str] = &[
    "chromium",
    "chromium-sandbox",
    "base-files",
    "bash",
    "dash",
    "coreutils",
    "findutils",
    "grep",
    "sed",
    "util-linux",
];

/// What the initramfs takes from the lock: a statically linked BusyBox for
/// its init (`/usr/bin/busybox` of Debian's `busybox-static`).
pub const BOOT_SEEDS: &[&str] = &["busybox-static"];

/// What only the reference needs, to observe Chromium on real Linux: a
/// Wayland compositor with a headless back end, an X server without a
/// screen, Xwayland, tools to click, type and take pictures of X windows,
/// and strace to trace Chromium inside the jail.
pub const REFERENCE_SEEDS: &[&str] = &["weston", "xvfb", "xwayland", "xdotool", "x11-apps", "xauth", "strace"];

/// The lock, relative to the repository.
pub const LOCK: &str = "tools/xtask/chromium.lock";

/// The disk image `cargo xtask chromium` boots (kept between runs).
pub const FILE_NAME: &str = "poly-chromium-disk.img";
/// The image the boot test makes afresh.
pub const TEST_FILE_NAME: &str = "poly-chromium-disk-test.img";

/// Where the test pages are on the disk and in the reference's root.
pub const PAGES_DIR: &str = "usr/share/poly/chromium";

/// The test pages: file name, contents. The scenario tests carry their own
/// copies in their `files`; these are for the boot test and for trying
/// things by hand.
pub const PAGES: [(&str, &str); 3] = [
    (
        "hello.html",
        "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>Poly Chromium hello</title></head>\n\
         <body><h1 id=\"h\">Hello from Poly</h1>\n\
         <p id=\"p\"></p>\n\
         <script>document.getElementById('p').textContent = 'script ran: ' + (6 * 7);</script>\n\
         </body></html>\n",
    ),
    (
        "colours.html",
        "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>Poly Chromium colours</title>\n\
         <style>html,body{margin:0;padding:0;overflow:hidden}\n\
         div{position:absolute;width:50vw;height:50vh}</style></head>\n\
         <body><div style=\"left:0;top:0;background:#ff0000\"></div>\
         <div style=\"left:50vw;top:0;background:#00ff00\"></div>\
         <div style=\"left:0;top:50vh;background:#0000ff\"></div>\
         <div style=\"left:50vw;top:50vh;background:#ffff00\"></div></body></html>\n",
    ),
    (
        "input.html",
        "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>Poly Chromium input</title>\n\
         <style>html,body{margin:0;padding:0;height:100%;overflow:hidden;background:#2a6fb0}\n\
         input{position:absolute;left:10px;top:10px;width:300px;height:30px;font-size:20px}</style></head>\n\
         <body><input id=\"t\" autofocus>\n\
         <script>\n\
         var clicks = 0;\n\
         document.addEventListener('mousedown', function (e) {\n\
           if (e.target.id === 't') return;\n\
           clicks++; document.body.style.background = clicks % 2 ? '#b03a2a' : '#2a6fb0';\n\
           document.title = 'clicked ' + clicks;\n\
         });\n\
         document.getElementById('t').addEventListener('input', function (e) {\n\
           document.title = 'typed ' + e.target.value;\n\
         });\n\
         </script></body></html>\n",
    ),
];

/// `input.html`'s background before and after a click.
pub const INPUT_BEFORE: [u8; 3] = [0x2a, 0x6f, 0xb0];
pub const INPUT_AFTER: [u8; 3] = [0xb0, 0x3a, 0x2a];

/// Chromium's arguments on Poly's desktop: Wayland (the desktop's
/// protocol, ADR 0124), no GPU process drawing with GL, no first-run
/// dialogs or network services a test does not want. The sandbox stays on.
pub const WINDOW_ARGS: &str = "--ozone-platform=wayland --disable-gpu --no-first-run --no-default-browser-check \
     --disable-background-networking --disable-component-update --disable-sync --password-store=basic";

/// `/usr/local/bin/poly-chromium`: Chromium as user `poly`, which it needs
/// to run its sandbox, on the desktop's Wayland display. The compositor's
/// runtime directory stays private (ADR 0120); the user gets a second name
/// of its socket (a hard link: unix(7) finds a socket by its inode) in its
/// own. `setpriv(1)` (util-linux) changes the user.
pub const LAUNCH_SCRIPT: &str = "#!/bin/bash
# Chromium as user poly on the desktop (tools/xtask/src/chromium.rs).
. /etc/poly-desktop.env
U=/run/user/1000
mkdir -p \"$U\" /home/poly
chown poly:poly \"$U\" /home/poly && chmod 700 \"$U\"
S=\"$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY\"
if [ -S \"$S\" ] && [ ! -e \"$U/$WAYLAND_DISPLAY\" ]; then
    ln \"$S\" \"$U/$WAYLAND_DISPLAY\" && chown poly:poly \"$U/$WAYLAND_DISPLAY\"
fi
cd /home/poly
exec setpriv --reuid=1000 --regid=1000 --init-groups env HOME=/home/poly XDG_RUNTIME_DIR=\"$U\" \\
    XDG_CONFIG_HOME=/home/poly/.config XDG_CACHE_HOME=/home/poly/.cache \\
    XDG_DATA_HOME=/home/poly/.local/share XDG_STATE_HOME=/home/poly/.local/state \\
    /usr/bin/chromium \"$@\"
";

/// `/etc/inittab` of Chromium's disk, read by BusyBox's init (from the disk,
/// `initramfs::DISK_BUSYBOX`) once the initramfs made the disk the root:
/// the desktop, the disk's bash on the console.
pub const INITTAB: &str = "\
# BusyBox init(8) table of Chromium's disk, the root (tools/xtask/src/chromium.rs)
::once:/usr/local/sbin/poly-desktop
::respawn:/usr/local/lib/poly/cttyhack /bin/bash -l
::ctrlaltdel:/usr/local/lib/poly/busybox reboot
::shutdown:/usr/local/lib/poly/busybox umount -a -r
";

/// `cargo xtask chromium` work directory.
fn work_dir() -> PathBuf {
    root().join("target").join("chromium")
}

/// One package of the lock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Locked {
    pub name: String,
    pub version: String,
    pub archive: String,
    pub size: u64,
    pub sha256: String,
    /// On Poly's disk (`disk`), in the initramfs (`boot`) or for the
    /// reference only (`reference`).
    pub set: String,
    pub filename: String,
}

impl Locked {
    fn url(&self) -> Result<String> {
        let a = ARCHIVES
            .iter()
            .find(|a| a.name == self.archive)
            .ok_or_else(|| format!("{}: no archive {}", self.name, self.archive))?;
        Ok(format!("{}/{}", a.base, self.filename))
    }

    /// The cached file's name (the pool file's own).
    fn file_name(&self) -> &str {
        self.filename.rsplit('/').next().unwrap_or(&self.filename)
    }

    fn on_disk(&self) -> bool {
        self.set == "disk"
    }
}

/// Parse the lock's text.
pub fn parse_lock(text: &str) -> Result<Vec<Locked>> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        let bad = || format!("{LOCK}:{}: name version archive size sha256 set filename expected", n + 1);
        if f.len() != 7 {
            return Err(bad());
        }
        let size = f[3].parse().map_err(|_| bad())?;
        if f[4].len() != 64
            || !f[4].bytes().all(|b| b.is_ascii_hexdigit())
            || !matches!(f[5], "disk" | "boot" | "reference")
        {
            return Err(bad());
        }
        let name_ok = f[6].bytes().all(|b| b.is_ascii_alphanumeric() || b"/._+~-".contains(&b));
        if !f[6].starts_with("pool/") || f[6].contains("..") || !f[6].ends_with(".deb") || !name_ok {
            return Err(bad());
        }
        out.push(Locked {
            name: f[0].into(),
            version: f[1].into(),
            archive: f[2].into(),
            size,
            sha256: f[4].into(),
            set: f[5].into(),
            filename: f[6].into(),
        });
    }
    Ok(out)
}

/// The committed lock.
pub fn lock() -> Result<Vec<Locked>> {
    let path = root().join(LOCK);
    parse_lock(&std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?)
}

// ---- the lock: package indexes and dependencies -------------------------

/// One stanza of a package index, the fields the lock needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stanza {
    pub package: String,
    pub version: String,
    pub filename: String,
    pub size: u64,
    pub sha256: String,
    pub depends: String,
    pub provides: String,
    pub archive: String,
}

/// Parse a Debian package index (deb-control(5) stanzas; `Pre-Depends`
/// counts as `Depends`).
pub fn parse_index(text: &str, archive: &str) -> Vec<Stanza> {
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let mut field = String::new();
        let mut fields: BTreeMap<String, String> = BTreeMap::new();
        for line in block.lines() {
            if line.starts_with(' ') || line.starts_with('\t') {
                if let Some(v) = fields.get_mut(&field) {
                    v.push(' ');
                    v.push_str(line.trim());
                }
            } else if let Some((k, v)) = line.split_once(':') {
                field = k.trim().to_string();
                fields.insert(field.clone(), v.trim().to_string());
            }
        }
        let get = |k: &str| fields.get(k).cloned().unwrap_or_default();
        let package = get("Package");
        if package.is_empty() {
            continue;
        }
        let depends =
            [get("Pre-Depends"), get("Depends")].into_iter().filter(|d| !d.is_empty()).collect::<Vec<_>>().join(", ");
        out.push(Stanza {
            package,
            version: get("Version"),
            filename: get("Filename"),
            size: get("Size").parse().unwrap_or(0),
            sha256: get("SHA256"),
            depends,
            provides: get("Provides"),
            archive: archive.into(),
        });
    }
    out
}

/// A relation field as groups of alternatives, package names only
/// (versions, architecture qualifiers and restrictions dropped: one
/// snapshot has one version of each package).
pub fn relations(field: &str) -> Vec<Vec<String>> {
    field
        .split(',')
        .map(|group| {
            group
                .split('|')
                .filter_map(|alt| {
                    let name = alt.trim().split([' ', '(', '[', '<']).next().unwrap_or("");
                    let name = name.split(':').next().unwrap_or("");
                    (!name.is_empty()).then(|| name.to_string())
                })
                .collect::<Vec<_>>()
        })
        .filter(|g| !g.is_empty())
        .collect()
}

/// The packages `seeds` need, following dependencies: for each group of
/// alternatives, one already chosen, else the first that is a package or
/// that a package provides (the first provider by name).
pub fn closure(seeds: &[&str], packages: &BTreeMap<String, Stanza>) -> Result<BTreeSet<String>> {
    let mut providers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for s in packages.values() {
        for group in relations(&s.provides) {
            for v in group {
                providers.entry(v).or_default().insert(s.package.clone());
            }
        }
    }
    let resolve = |name: &str| -> Option<String> {
        if packages.contains_key(name) {
            Some(name.to_string())
        } else {
            providers.get(name).and_then(|p| p.iter().next().cloned())
        }
    };
    let mut chosen = BTreeSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    for seed in seeds {
        let p = resolve(seed).ok_or_else(|| format!("package {seed}: not in the snapshots"))?;
        if chosen.insert(p.clone()) {
            queue.push_back(p);
        }
    }
    while let Some(p) = queue.pop_front() {
        let s = &packages[&p];
        for group in relations(&s.depends) {
            let satisfied = group.iter().any(|alt| {
                chosen.contains(alt) || providers.get(alt).is_some_and(|ps| ps.iter().any(|x| chosen.contains(x)))
            });
            if satisfied {
                continue;
            }
            let pick = group
                .iter()
                .find_map(|alt| resolve(alt))
                .ok_or_else(|| format!("{p}: no package for {}", group.join(" | ")))?;
            if chosen.insert(pick.clone()) {
                queue.push_back(pick);
            }
        }
    }
    Ok(chosen)
}

/// Download `url` to `to` with curl (HTTPS only, like `fonts.rs`).
fn download(url: &str, to: &Path) -> Result {
    if !url.starts_with("https://") {
        return Err(format!("{url}: not HTTPS"));
    }
    let partial = to.with_extension("part");
    let ok = Command::new("curl")
        .args(["-fsSL", "--retry", "3", "-o"])
        .arg(&partial)
        .arg(url)
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        let _ = std::fs::remove_file(&partial);
        return Err(format!("downloading {url} failed (curl)"));
    }
    std::fs::rename(&partial, to).map_err(|e| format!("{}: {e}", to.display()))
}

/// `path` has `size` bytes and the SHA-256 `sum`.
fn matches(path: &Path, size: u64, sum: &str) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.len() == size) && crate::sha256::file(path).as_deref() == Some(sum)
}

/// Decompress an xz stream (xz(1)'s format) in pure Rust.
pub fn unxz(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    lzma_rs::xz_decompress(&mut std::io::BufReader::new(data), &mut out).map_err(|e| format!("xz: {e:?}"))?;
    Ok(out)
}

/// The snapshot's package index, downloaded, checked and decompressed.
fn index_text(a: &Archive) -> Result<String> {
    let dir = work_dir().join("indexes");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join(format!("{}-Packages.xz", a.name));
    if !matches(&file, a.size, a.sha256) {
        let url = format!("{}/{}", a.base, a.index);
        eprintln!("==> downloading {url}");
        download(&url, &file)?;
        if !matches(&file, a.size, a.sha256) {
            return Err(format!("{}: not the pinned index (size {}, SHA-256 {})", file.display(), a.size, a.sha256));
        }
    }
    let data = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    String::from_utf8(unxz(&data)?).map_err(|_| format!("{}: not UTF-8", file.display()))
}

/// `cargo xtask chromium lock`: write [`LOCK`] from the pinned indexes.
fn write_lock() -> Result {
    let mut packages: BTreeMap<String, Stanza> = BTreeMap::new();
    for a in &ARCHIVES {
        for s in parse_index(&index_text(a)?, a.name) {
            packages.insert(s.package.clone(), s);
        }
    }
    let disk = closure(DISK_SEEDS, &packages)?;
    let boot = closure(BOOT_SEEDS, &packages)?;
    let all: Vec<&str> = DISK_SEEDS.iter().chain(BOOT_SEEDS).chain(REFERENCE_SEEDS).copied().collect();
    let everything = closure(&all, &packages)?;
    let mut text = String::from(
        "# Debian packages of Chromium's root disk, its initramfs's BusyBox and the reference\n\
         # Chromium is observed in (tools/xtask/src/chromium.rs). Written by\n\
         # `cargo xtask chromium lock` from the pinned snapshots' indexes; do not edit by hand.\n\
         # name version archive size sha256 set filename\n",
    );
    for name in &everything {
        let s = &packages[name];
        if s.sha256.len() != 64 || s.filename.is_empty() {
            return Err(format!("{name}: no SHA-256 or file name in the index"));
        }
        let set = if disk.contains(name) {
            "disk"
        } else if boot.contains(name) {
            "boot"
        } else {
            "reference"
        };
        text.push_str(&format!(
            "{} {} {} {} {} {set} {}\n",
            s.package, s.version, s.archive, s.size, s.sha256, s.filename
        ));
    }
    let path = root().join(LOCK);
    std::fs::write(&path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
    let bytes = |set: &BTreeSet<String>| set.iter().map(|n| packages[n].size).sum::<u64>() >> 20;
    eprintln!(
        "==> {}: {} packages ({} MiB), {} of them on the disk ({} MiB)",
        path.display(),
        everything.len(),
        bytes(&everything),
        disk.len(),
        bytes(&disk)
    );
    Ok(())
}

// ---- fetching ------------------------------------------------------------

/// Where the downloaded packages are kept.
fn debs_dir() -> PathBuf {
    work_dir().join("debs")
}

/// The locked packages of `sets` that are not in the cache (or not as
/// locked).
fn missing<'a>(locked: &'a [Locked], sets: &[&str]) -> Vec<&'a Locked> {
    let dir = debs_dir();
    locked
        .iter()
        .filter(|p| sets.contains(&p.set.as_str()))
        .filter(|p| !matches(&dir.join(p.file_name()), p.size, &p.sha256))
        .collect()
}

/// The sets every host needs (the reference is the spec agent's).
const BUILD_SETS: [&str; 2] = ["disk", "boot"];

/// `cargo xtask chromium fetch [--reference]`: download what is missing
/// of the disk's and the initramfs's packages (and the reference's), check
/// every one.
pub fn fetch(reference: bool) -> Result {
    let locked = lock()?;
    let dir = debs_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let sets: &[&str] = if reference { &["disk", "boot", "reference"] } else { &BUILD_SETS };
    let todo = missing(&locked, sets);
    let wanted = locked.iter().filter(|p| sets.contains(&p.set.as_str())).count();
    eprintln!("==> {} of {wanted} locked packages to download", todo.len());
    for p in todo {
        let file = dir.join(p.file_name());
        download(&p.url()?, &file)?;
        if !matches(&file, p.size, &p.sha256) {
            let _ = std::fs::remove_file(&file);
            return Err(format!("{}: not as locked (size {}, SHA-256 {})", p.file_name(), p.size, p.sha256));
        }
    }
    eprintln!("==> {wanted} packages in {} as locked", dir.display());
    Ok(())
}

// ---- reading packages in pure Rust -----------------------------------------

/// The members of an `ar(5)` archive (a `.deb`, deb(5)): name and data.
pub fn ar_members(data: &[u8]) -> Result<Vec<(String, &[u8])>> {
    let rest = data.strip_prefix(b"!<arch>\n").ok_or("not an ar archive")?;
    let mut out = Vec::new();
    let mut at = 0;
    while at + 60 <= rest.len() {
        let h = &rest[at..at + 60];
        if &h[58..60] != b"`\n" {
            return Err(format!("ar: bad member header at {}", at + 8));
        }
        let name = String::from_utf8_lossy(&h[..16]).trim_end().trim_end_matches('/').to_string();
        let size: usize = String::from_utf8_lossy(&h[48..58]).trim().parse().map_err(|_| "ar: bad member size")?;
        let start = at + 60;
        let body = rest.get(start..start + size).ok_or("ar: truncated member")?;
        out.push((name, body));
        at = start + size + (size & 1);
    }
    Ok(out)
}

/// What a tar entry is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TarKind {
    File(Vec<u8>),
    Dir,
    Symlink(String),
    /// A hard link to an earlier entry (its path).
    Link(String),
}

/// One tar entry: path (no leading `./` or `/`), permission bits, time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TarEntry {
    pub path: String,
    pub mode: u16,
    pub mtime: i64,
    pub kind: TarKind,
}

fn octal(field: &[u8]) -> u64 {
    let s = String::from_utf8_lossy(field);
    u64::from_str_radix(s.trim_matches(|c: char| c == '\0' || c == ' '), 8).unwrap_or(0)
}

fn cstr(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

fn clean_path(p: &str) -> String {
    p.trim_start_matches("./").trim_start_matches('/').trim_end_matches('/').to_string()
}

/// The entries of a tar archive (POSIX ustar, with GNU long names and pax
/// `path`/`linkpath` records, as dpkg writes them).
pub fn tar_entries(data: &[u8]) -> Result<Vec<TarEntry>> {
    let mut out = Vec::new();
    let mut at = 0;
    let (mut long_name, mut long_link): (Option<String>, Option<String>) = (None, None);
    while at + 512 <= data.len() {
        let h = &data[at..at + 512];
        if h.iter().all(|&b| b == 0) {
            break;
        }
        let size = octal(&h[124..136]) as usize;
        let body = data.get(at + 512..at + 512 + size).ok_or("tar: truncated entry")?;
        at += 512 + size.div_ceil(512) * 512;
        let typ = h[156];
        match typ {
            b'L' => {
                long_name = Some(cstr(body));
                continue;
            }
            b'K' => {
                long_link = Some(cstr(body));
                continue;
            }
            b'x' => {
                // pax records: "<len> key=value\n".
                let text = String::from_utf8_lossy(body);
                for rec in text.split('\n') {
                    if let Some((_, kv)) = rec.split_once(' ')
                        && let Some((k, v)) = kv.split_once('=')
                    {
                        match k {
                            "path" => long_name = Some(v.to_string()),
                            "linkpath" => long_link = Some(v.to_string()),
                            _ => {}
                        }
                    }
                }
                continue;
            }
            b'g' => continue,
            _ => {}
        }
        let mut name = cstr(&h[0..100]);
        let prefix = cstr(&h[345..500]);
        if &h[257..262] == b"ustar" && !prefix.is_empty() {
            name = format!("{prefix}/{name}");
        }
        let path = clean_path(&long_name.take().unwrap_or(name));
        let link = long_link.take().unwrap_or_else(|| cstr(&h[157..257]));
        let mode = (octal(&h[100..108]) & 0o7777) as u16;
        let mtime = octal(&h[136..148]) as i64;
        let kind = match typ {
            b'0' | 0 | b'7' => TarKind::File(body.to_vec()),
            b'5' => TarKind::Dir,
            b'2' => TarKind::Symlink(link),
            b'1' => TarKind::Link(clean_path(&link)),
            // Devices and FIFOs: none in the packages Chromium needs.
            other => return Err(format!("tar: {path}: entry type {:?} not supported", other as char)),
        };
        if path.is_empty() {
            continue;
        }
        out.push(TarEntry { path, mode, mtime, kind });
    }
    Ok(out)
}

/// The entries of a `.deb`'s file system tree (`data.tar.xz`).
pub fn deb_entries(deb: &[u8]) -> Result<Vec<TarEntry>> {
    let members = ar_members(deb)?;
    let (name, data) =
        members.iter().find(|(n, _)| n.starts_with("data.tar")).ok_or("no data.tar member (not a .deb?)")?;
    let tar = match name.as_str() {
        "data.tar.xz" => unxz(data)?,
        "data.tar" => data.to_vec(),
        other => return Err(format!("{other}: compression not supported (Debian 13 uses xz)")),
    };
    tar_entries(&tar)
}

/// Debian 13's merged `/usr`: the top-level directories are links into
/// it, and a package's file under one of them lands below `/usr`.
const USR_LINKS: [(&str, &str); 4] =
    [("bin", "usr/bin"), ("sbin", "usr/sbin"), ("lib", "usr/lib"), ("lib64", "usr/lib64")];

fn merged_usr(path: &str) -> String {
    for (link, target) in USR_LINKS {
        if path == link {
            return target.to_string();
        }
        if let Some(rest) = path.strip_prefix(link).and_then(|r| r.strip_prefix('/')) {
            return format!("{target}/{rest}");
        }
    }
    path.to_string()
}

/// One entry of the unpacked disk set.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Unpacked {
    Dir {
        path: String,
        mode: u16,
    },
    /// A regular file whose contents are blob `blob`.
    File {
        path: String,
        mode: u16,
        mtime: i64,
        blob: usize,
    },
    Symlink {
        path: String,
        target: String,
    },
}

/// Where the disk set is unpacked: `blobs/<n>` and `index.txt`.
fn unpack_dir() -> PathBuf {
    work_dir().join("disk")
}

/// The index's first line: which lock it is from.
fn index_stamp() -> Result<String> {
    let text = std::fs::read(root().join(LOCK)).map_err(|e| format!("{LOCK}: {e}"))?;
    Ok(format!("poly-chromium-index/1 {}", crate::sha256::hex(&text)))
}

fn write_index(path: &Path, stamp: &str, entries: &[Unpacked]) -> Result {
    let mut out = format!("{stamp}\n");
    for e in entries {
        match e {
            Unpacked::Dir { path, mode } => out.push_str(&format!("d\t{mode:o}\t{path}\n")),
            Unpacked::File { path, mode, mtime, blob } => {
                out.push_str(&format!("f\t{mode:o}\t{mtime}\t{blob}\t{path}\n"))
            }
            Unpacked::Symlink { path, target } => out.push_str(&format!("l\t{target}\t{path}\n")),
        }
    }
    std::fs::write(path, out).map_err(|e| format!("{}: {e}", path.display()))
}

fn read_index(path: &Path, stamp: &str) -> Option<Vec<Unpacked>> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    if lines.next()? != stamp {
        return None;
    }
    let mut out = Vec::new();
    for line in lines {
        let f: Vec<&str> = line.split('\t').collect();
        out.push(match f.as_slice() {
            ["d", mode, path] => Unpacked::Dir { path: path.to_string(), mode: u16::from_str_radix(mode, 8).ok()? },
            ["f", mode, mtime, blob, path] => Unpacked::File {
                path: path.to_string(),
                mode: u16::from_str_radix(mode, 8).ok()?,
                mtime: mtime.parse().ok()?,
                blob: blob.parse().ok()?,
            },
            ["l", target, path] => Unpacked::Symlink { path: path.to_string(), target: target.to_string() },
            _ => return None,
        });
    }
    Some(out)
}

/// Unpack the `disk` set in pure Rust (once per lock): the entries, their
/// regular files as numbered blobs.
fn unpack_disk() -> Result<Vec<Unpacked>> {
    let locked = lock()?;
    let absent = missing(&locked, &["disk"]);
    if !absent.is_empty() {
        return Err(format!("{} locked packages are not downloaded: run `cargo xtask chromium fetch`", absent.len()));
    }
    let dir = unpack_dir();
    let index = dir.join("index.txt");
    let stamp = index_stamp()?;
    if let Some(entries) = read_index(&index, &stamp) {
        return Ok(entries);
    }
    let _ = std::fs::remove_dir_all(&dir);
    let blobs = dir.join("blobs");
    std::fs::create_dir_all(&blobs).map_err(|e| format!("{}: {e}", blobs.display()))?;
    let mut entries = Vec::new();
    // Path of each regular file so far -> its blob (for hard links).
    let mut files: BTreeMap<String, usize> = BTreeMap::new();
    let wanted: Vec<&Locked> = locked.iter().filter(|p| p.on_disk()).collect();
    eprintln!("==> unpacking {} packages (pure Rust) into {}", wanted.len(), dir.display());
    for p in wanted {
        let file = debs_dir().join(p.file_name());
        let deb = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        for e in deb_entries(&deb).map_err(|e| format!("{}: {e}", p.file_name()))? {
            let path = merged_usr(&e.path);
            if USR_LINKS.iter().any(|(l, _)| *l == path) {
                continue;
            }
            match e.kind {
                TarKind::Dir => entries.push(Unpacked::Dir { path, mode: e.mode }),
                TarKind::Symlink(target) => entries.push(Unpacked::Symlink { path, target }),
                TarKind::File(data) => {
                    let blob = files.len();
                    let at = blobs.join(blob.to_string());
                    std::fs::File::create(&at)
                        .and_then(|mut f| f.write_all(&data))
                        .map_err(|e| format!("{}: {e}", at.display()))?;
                    files.insert(path.clone(), blob);
                    entries.push(Unpacked::File { path, mode: e.mode, mtime: e.mtime, blob });
                }
                TarKind::Link(to) => {
                    let blob =
                        *files.get(&merged_usr(&to)).ok_or_else(|| format!("{path}: hard link to unknown {to}"))?;
                    files.insert(path.clone(), blob);
                    entries.push(Unpacked::File { path, mode: e.mode, mtime: e.mtime, blob });
                }
            }
        }
    }
    write_index(&index, &stamp, &entries)?;
    Ok(entries)
}

/// A file of a package of the lock (e.g. `usr/bin/busybox` of
/// `busybox-static`), read in pure Rust.
fn package_file(package: &str, path: &str) -> Result<Vec<u8>> {
    let locked = lock()?;
    let p = locked.iter().find(|p| p.name == package).ok_or_else(|| format!("{package}: not in {LOCK}"))?;
    let file = debs_dir().join(p.file_name());
    if !matches(&file, p.size, &p.sha256) {
        return Err(format!("{package} is not downloaded: run `cargo xtask chromium fetch`"));
    }
    let deb = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    deb_entries(&deb)?
        .into_iter()
        .find(|e| merged_usr(&e.path) == path)
        .and_then(|e| match e.kind {
            TarKind::File(data) => Some(data),
            _ => None,
        })
        .ok_or_else(|| format!("{package}: no file {path}"))
}

// ---- the reference (Linux only) --------------------------------------------

/// Unpack every locked package with `dpkg-deb -x` into
/// `target/chromium/root-reference/` (real links and set-user-ID modes on
/// the host: the tree Chromium is observed in on Linux).
pub fn unpack_reference() -> Result<PathBuf> {
    if !cfg!(target_os = "linux") {
        return Err("the reference is observed on Linux (dpkg-deb, chroot)".into());
    }
    let locked = lock()?;
    let absent = missing(&locked, &["disk", "boot", "reference"]);
    if !absent.is_empty() {
        return Err(format!(
            "{} locked packages are not downloaded: run `cargo xtask chromium fetch --reference`",
            absent.len()
        ));
    }
    let dir = work_dir().join("root-reference");
    let stamp_text = index_stamp()?;
    let stamp = dir.join(".poly-lock");
    if !std::fs::read_to_string(&stamp).is_ok_and(|s| s == stamp_text) {
        unpack_reference_packages(&locked, &dir)?;
        std::fs::write(&stamp, stamp_text).map_err(|e| format!("{}: {e}", stamp.display()))?;
    }
    // The configuration and the test pages, written every time.
    let mut tree = RootTree::default();
    configure(&mut tree);
    for (path, entry) in &tree.entries {
        let at = dir.join(path);
        match entry {
            rootdisk::Entry::Dir(mode) => {
                std::fs::create_dir_all(&at).map_err(|e| format!("{}: {e}", at.display()))?;
                crate::hostfs::set_mode(&at, u32::from(*mode)).map_err(|e| format!("{}: {e}", at.display()))?;
            }
            rootdisk::Entry::File { mode, source: rootdisk::Source::Bytes(b), .. } => {
                std::fs::write(&at, b).map_err(|e| format!("{}: {e}", at.display()))?;
                crate::hostfs::set_mode(&at, u32::from(*mode)).map_err(|e| format!("{}: {e}", at.display()))?;
            }
            _ => {}
        }
    }
    Ok(dir)
}

/// Unpack every locked package into `dir` (afresh), `dpkg-deb -x`.
fn unpack_reference_packages(locked: &[Locked], dir: &Path) -> Result {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join("usr")).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (link, target) in USR_LINKS {
        std::fs::create_dir_all(dir.join(target)).map_err(|e| format!("{target}: {e}"))?;
        crate::hostfs::symlink(target, &dir.join(link)).map_err(|e| format!("{link}: {e}"))?;
    }
    eprintln!("==> unpacking {} packages into {}", locked.len(), dir.display());
    for p in locked {
        let out = Command::new("dpkg-deb")
            .arg("-x")
            .arg(debs_dir().join(p.file_name()))
            .arg(dir)
            .output()
            .map_err(|e| format!("dpkg-deb: {e}"))?;
        if !out.status.success() {
            return Err(format!("dpkg-deb -x {}: {}", p.file_name(), String::from_utf8_lossy(&out.stderr).trim()));
        }
    }
    Ok(())
}

// ---- the disk --------------------------------------------------------------

/// The users: root, and `poly` (uid 1000), whom Chromium runs as.
pub const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\n\
                          nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin\n\
                          poly:x:1000:1000:Poly:/home/poly:/bin/bash\n";
pub const GROUP: &str = "root:x:0:\nnogroup:x:65534:\npoly:x:1000:\n";

/// What maintainer scripts would have done that Chromium needs, the users
/// and the test pages; the same on the disk and in the reference.
fn configure(tree: &mut RootTree) {
    tree.file("etc/passwd", 0o644, PASSWD.as_bytes());
    tree.file("etc/group", 0o644, GROUP.as_bytes());
    // A fixed machine id (machine-id(5)): D-Bus and Chromium read it.
    tree.file("etc/machine-id", 0o444, b"504f4c59636872006f6d69756d000001\n");
    for (path, data) in PAGES {
        tree.file(&format!("{PAGES_DIR}/{path}"), 0o644, data.as_bytes());
    }
    for d in ["proc", "sys", "dev", "run", "root"] {
        tree.dir(d, 0o755);
    }
    tree.dir("tmp", 0o1777);
    tree.dir("home/poly", 0o755);
}

/// The launcher's desktop file for Chromium: as `poly`, sandbox on.
fn desktop_file() -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Chromium\nComment=Web browser (Debian's Chromium)\n\
         Exec=/usr/local/bin/poly-chromium {WINDOW_ARGS} file:///{PAGES_DIR}/input.html\n"
    )
}

/// Collect Chromium's disk: the unpacked packages and the desktop.
pub fn collect() -> std::result::Result<Contents, String> {
    let entries = unpack_disk()?;
    let blobs = unpack_dir().join("blobs");
    let mut contents = Contents::default();
    let tree = &mut contents.tree;
    for (link, target) in USR_LINKS {
        tree.dir(target, 0o755);
        tree.symlink(link, target);
    }
    for e in entries {
        match e {
            Unpacked::Dir { path, mode } => tree.dir(&path, mode),
            Unpacked::Symlink { path, target } => tree.symlink(&path, &target),
            Unpacked::File { path, mode, mtime, blob } => {
                tree.host_file(&path, mode, &blobs.join(blob.to_string()), mtime);
            }
        }
    }
    contents.have.insert("chromium");
    configure(tree);
    tree.file("usr/local/bin/poly-compositor", 0o755, &compositor::build_compositor()?);
    for (path, data) in crate::fonts::shell_tree() {
        tree.file(&path, 0o644, &data);
    }
    for (path, data) in crate::wallpapers::tree().map_err(|e| e.to_string())? {
        tree.file(&path, 0o644, &data);
    }
    tree.file("etc/poly-desktop.env", 0o644, crate::toolkits::DESKTOP_ENV.as_bytes());
    let profile = format!("{}. /etc/poly-desktop.env\n", rootdisk::PROFILE);
    tree.file("etc/profile", 0o644, profile.as_bytes());
    tree.file("usr/local/sbin/poly-desktop", 0o755, crate::toolkits::DESKTOP_SCRIPT.as_bytes());
    tree.file("usr/local/bin/poly-chromium", 0o755, LAUNCH_SCRIPT.as_bytes());
    // Debian's poweroff and reboot are systemd's (systemd-sysv comes with
    // the dependencies), which is not running: the root disk's own, first
    // in PATH (ADR 0059), signal BusyBox's init.
    tree.file("usr/local/sbin/poweroff", 0o755, rootdisk::POWEROFF.as_bytes());
    tree.file("usr/local/sbin/reboot", 0o755, rootdisk::REBOOT.as_bytes());
    // BusyBox's init is pid 1 on the disk (the initramfs switched to it as
    // the root), with its table and the applets the table runs.
    tree.file(crate::initramfs::DISK_BUSYBOX, 0o755, &package_file("busybox-static", "usr/bin/busybox")?);
    tree.symlink("usr/local/lib/poly/cttyhack", "busybox");
    tree.file("etc/inittab", 0o644, INITTAB.as_bytes());
    tree.file("usr/share/applications/poly-chromium.desktop", 0o644, desktop_file().as_bytes());
    for d in ["root/.config", "root/.cache", "root/.local/share", "run/user"] {
        tree.dir(d, 0o755);
    }
    Ok(contents)
}

/// Collect the disk and write it to `target/<name>` with `poly-ext2`'s
/// formatter (the same image on every host).
pub fn make(name: &str) -> std::result::Result<(PathBuf, Contents), String> {
    let contents = collect()?;
    for note in &contents.notes {
        eprintln!("==> note: chromium disk: {note}");
    }
    let path = root().join("target").join(name);
    rootdisk::write_image_poly_ext2(&contents.tree, &path)?;
    eprintln!("==> chromium disk {} (poly-ext2's format): {} entries", path.display(), contents.tree.entries.len());
    Ok((path, contents))
}

/// The initramfs: BusyBox (Debian's `busybox-static` from the lock), whose
/// shell is the first program: it mounts the disk and switches to it as the
/// root (`initramfs::SWITCH_ROOT_SCRIPT`), where BusyBox's init runs
/// [`INITTAB`]. Booted with [`INIT_ARGS`].
pub fn write_initramfs() -> Result<PathBuf> {
    let busybox = package_file("busybox-static", "usr/bin/busybox")?;
    let image = crate::initramfs::switch_root_image(&busybox);
    crate::initramfs::write(&root(), "poly-chromium-initramfs.cpio", &image)
}

/// The kernel arguments naming the first program of Chromium's boot.
pub const INIT_ARGS: &str = crate::initramfs::SWITCH_ROOT_INIT_ARGS;

/// Guest memory: Chromium's browser, zygotes, renderer and utility
/// processes side by side.
pub const MEMORY: &str = "4G";

/// Virtual CPUs of Chromium's boots: its processes run side by side (QEMU's
/// TCG emulates each CPU on a host thread of its own).
pub const CPUS: Option<u32> = Some(4);

/// `cargo xtask chromium run [--release] [--fresh-disk]`: the desktop in
/// QEMU's window with Chromium in the launcher, the disk's bash on this
/// terminal (Ctrl-A X quits). Any host.
fn run(args: &[String]) -> Result {
    let release = args.iter().any(|a| a == "--release");
    let kernel = crate::build_kernel(release)?;
    let image = root().join("target").join(FILE_NAME);
    if args.iter().any(|a| a == "--fresh-disk") || !image.exists() {
        make(FILE_NAME)?;
    }
    let initrd = write_initramfs()?;
    let append = format!("{INIT_ARGS} console=ttyS0 poly.terminal=off");
    let mut cmd = crate::qemu(&kernel, MEMORY, &append, crate::Devices::None, "mon:stdio");
    cmd.args(rootdisk::drive_args(&image)).arg("-initrd").arg(&initrd).args(crate::desktop::devices());
    if let Some(n) = CPUS {
        cmd.args(["-smp", &n.to_string()]);
    }
    eprintln!(
        "==> the desktop is in QEMU's window (the Windows key or Ctrl+Space opens the launcher: type \"chromium\"); \
         the disk's bash is here (Ctrl-A X quits)"
    );
    // QEMU's GTK window on Windows would shrink the screen to its scaled
    // size (desktop::window_display).
    let _ = crate::desktop::window_display(&mut cmd);
    let _ = cmd.status().map_err(|e| format!("qemu: {e}"))?;
    Ok(())
}

/// `cargo xtask chromium <lock|fetch [--reference]|disk|run|root
/// --reference>` or `cargo xtask chromium [--release] [--fresh-disk]
/// [STEP...]` (a boot driven by steps as `cargo xtask toolkits`).
pub fn command(args: &[String]) -> Result {
    match args.first().map(String::as_str) {
        Some("lock") => return write_lock(),
        Some("fetch") => return fetch(args.iter().any(|a| a == "--reference")),
        Some("root") => {
            let dir = unpack_reference()?;
            println!("{}", dir.display());
            return Ok(());
        }
        Some("disk") => {
            fetch(false)?;
            let (path, _) = make(FILE_NAME)?;
            println!("{}", path.display());
            return Ok(());
        }
        Some("run") => {
            fetch(false)?;
            return run(&args[1..]);
        }
        Some("apptest") => return apptest(&args[1..]),
        _ => {}
    }
    if !cfg!(unix) {
        return Err("a driven boot needs a Unix host (QMP); `cargo xtask chromium run` boots on any host".into());
    }
    let release = args.iter().any(|a| a == "--release");
    let fresh = args.iter().any(|a| a == "--fresh-disk");
    let steps = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(|a| crate::toolkits::Step::parse(a))
        .collect::<Result<Vec<_>>>()?;
    fetch(false)?;
    let kernel = crate::build_kernel(release)?;
    let syms = Arc::new(Symbols::from_file(&kernel)?);
    let mut image = root().join("target").join(FILE_NAME);
    // POLY_CHROMIUM_PROBE_DIR: a directory of scripts to try things with,
    // copied to /root/probe/ on a fresh image of its own.
    if let Ok(dir) = std::env::var("POLY_CHROMIUM_PROBE_DIR") {
        let mut contents = collect()?;
        for e in std::fs::read_dir(&dir).map_err(|e| format!("{dir}: {e}"))?.flatten() {
            let data = std::fs::read(e.path()).map_err(|e| e.to_string())?;
            contents.tree.file(&format!("root/probe/{}", e.file_name().to_string_lossy()), 0o755, &data);
        }
        image = root().join("target/poly-chromium-probe.img");
        rootdisk::write_image_poly_ext2(&contents.tree, &image)?;
    } else if fresh || !image.exists() {
        make(FILE_NAME)?;
    }
    let initrd = write_initramfs()?;
    let mut m = crate::toolkits::Machine::boot_with_init(
        &kernel,
        &syms,
        &image,
        &initrd,
        "chromium-probe",
        MEMORY,
        INIT_ARGS,
        CPUS,
    )?;
    m.set_timeouts(Duration::from_secs(3600), Duration::from_secs(4 * 3600));
    m.desktop_up()?;
    let mut result = Ok(());
    for step in &steps {
        if let Err(e) = m.step(step) {
            result = Err(format!("{step:?}: {e}"));
            break;
        }
    }
    let log = root().join("target/poly-chromium-probe.log");
    let _ = std::fs::write(&log, m.text());
    eprintln!("==> the console's output is in {}", log.display());
    let off = m.poweroff();
    result.and(off)
}

// ---- scenario tests (`specs/linux/apps/chromium.tests/*.app`) ---------------

/// Where Chromium's visible scenario tests are.
pub const TESTS_DIR: &str = "specs/linux/apps/chromium.tests";

/// The environment of every step (as the pipeline's app tests, ADR 0057,
/// with `/usr/bin` for Debian's merged `/usr`), before the test's `env`.
pub const TEST_ENV: &[&str] = &["PATH=/usr/bin:/bin", "HOME=/work", "LANG=C", "TZ=UTC"];

/// One scenario test, in the pipeline's app test format
/// (docs/AGENT_PIPELINE.md, "App tests": `description`, `steps`, `stdin`,
/// `env`, `files`), run on Chromium's disk instead of a tree of host
/// programs: the steps name programs of the disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppTest {
    pub name: String,
    pub description: String,
    pub steps: Vec<Vec<String>>,
    pub stdin: String,
    pub env: Vec<String>,
    /// (path under /work, contents or `None` for a directory, mode).
    pub files: Vec<(String, Option<String>, u32)>,
}

fn str_list(v: &serde_json::Value, what: &str) -> Result<Vec<String>> {
    v.as_array()
        .ok_or_else(|| format!("{what}: a list expected"))?
        .iter()
        .map(|s| s.as_str().map(str::to_string).ok_or_else(|| format!("{what}: strings expected")))
        .collect()
}

impl AppTest {
    pub fn parse(name: &str, text: &str) -> Result<AppTest> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("{name}: {e}"))?;
        let obj = v.as_object().ok_or_else(|| format!("{name}: an object expected"))?;
        for k in obj.keys() {
            if !["description", "steps", "stdin", "env", "files"].contains(&k.as_str()) {
                return Err(format!("{name}: unknown field {k:?}"));
            }
        }
        let steps = obj
            .get("steps")
            .and_then(|s| s.as_array())
            .ok_or_else(|| format!("{name}: no steps"))?
            .iter()
            .map(|s| str_list(s, "step"))
            .collect::<Result<Vec<_>>>()?;
        if steps.is_empty() || steps.iter().any(|s| s.is_empty() || s[0].contains('/')) {
            return Err(format!("{name}: steps must be non-empty, each naming a program (no path)"));
        }
        let mut files = Vec::new();
        for f in obj.get("files").and_then(|f| f.as_array()).into_iter().flatten() {
            let path = f.get("path").and_then(|p| p.as_str()).ok_or_else(|| format!("{name}: a file without path"))?;
            if path.starts_with('/') || path.split('/').any(|c| c.is_empty() || c == "..") {
                return Err(format!("{name}: file {path:?}: a relative path inside /work expected"));
            }
            let dir = f.get("dir").and_then(|d| d.as_bool()).unwrap_or(false);
            let text = f.get("text").and_then(|t| t.as_str()).map(str::to_string);
            if dir == text.is_some() || f.get("symlink").is_some() {
                return Err(format!("{name}: file {path:?}: exactly one of text and dir (no symlinks here)"));
            }
            let mode = match f.get("mode").and_then(|m| m.as_str()) {
                Some(m) => u32::from_str_radix(m, 8).map_err(|_| format!("{name}: bad mode {m:?}"))?,
                None if dir => 0o755,
                None => 0o644,
            };
            files.push((path.to_string(), text, mode));
        }
        Ok(AppTest {
            name: name.to_string(),
            description: obj.get("description").and_then(|d| d.as_str()).unwrap_or_default().to_string(),
            steps,
            stdin: obj.get("stdin").and_then(|s| s.as_str()).unwrap_or_default().to_string(),
            env: obj.get("env").map(|e| str_list(e, "env")).transpose()?.unwrap_or_default(),
            files,
        })
    }

    /// The test as a bash script the same on both systems: the files under
    /// a fresh `/work`, every step with the fixed environment and stdin,
    /// stdout of all steps into one file, then two marker lines: stdout in
    /// hex, and the last step's exit status. Stderr is not compared:
    /// Chromium's log lines carry pids and times.
    pub fn script(&self) -> String {
        let q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
        let mut s = format!(
            "#!/bin/bash\n# poly-chromium-apptest/1 {}\nT=/tmp/poly-apptest\nrm -rf /work \"$T\"\nmkdir -p /work \"$T\" /tmp/poly-home\n\
             chown 1000:1000 /tmp/poly-home\ncd /work\numask 022\n",
            self.name
        );
        for (path, text, mode) in &self.files {
            match text {
                None => s.push_str(&format!("mkdir -p {}\n", q(path))),
                Some(t) => {
                    if let Some((parent, _)) = path.rsplit_once('/') {
                        s.push_str(&format!("mkdir -p {}\n", q(parent)));
                    }
                    s.push_str(&format!("echo {} | base64 -d > {}\n", base64(t.as_bytes()), q(path)));
                }
            }
            s.push_str(&format!("chmod {mode:o} {}\n", q(path)));
        }
        s.push_str(&format!("echo {} | base64 -d > \"$T/stdin\"\n: > \"$T/out\"\n", base64(self.stdin.as_bytes())));
        let env: Vec<String> = TEST_ENV.iter().map(|e| e.to_string()).chain(self.env.iter().cloned()).collect();
        let env = env.iter().map(|e| q(e)).collect::<Vec<_>>().join(" ");
        for step in &self.steps {
            let argv = step.iter().map(|a| q(a)).collect::<Vec<_>>().join(" ");
            s.push_str(&format!("env -i {env} {argv} < \"$T/stdin\" >> \"$T/out\" 2>> \"$T/err\"\nstatus=$?\n"));
        }
        s.push_str("echo \"POLY-APPTEST-OUT $(od -An -v -tx1 \"$T/out\" | tr -d ' \\n')\"\n");
        s.push_str("echo \"POLY-APPTEST-STATUS $status\"\n");
        s
    }
}

/// Standard base64 (RFC 4648), for the scripts' files.
fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n =
            (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    if out.is_empty() { "''".into() } else { out }
}

/// What a run of a test gave: stdout and the exit status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub stdout: Vec<u8>,
    pub status: String,
}

/// The outcome in a run's output (the marker lines of [`AppTest::script`]).
pub fn outcome(text: &str) -> Option<Outcome> {
    let mut hex = None;
    let mut status = None;
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(h) = line.strip_prefix("POLY-APPTEST-OUT ") {
            hex = Some(h.trim().to_string());
        } else if let Some(s) = line.strip_prefix("POLY-APPTEST-STATUS ") {
            status = Some(s.trim().to_string());
        }
    }
    let hex = hex?;
    let stdout = (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16))
        .collect::<std::result::Result<_, _>>()
        .ok()?;
    Some(Outcome { stdout, status: status? })
}

/// The tests of `dirs` (`*.app`), by name.
fn load_tests(dirs: &[PathBuf]) -> Result<Vec<AppTest>> {
    let mut out = Vec::new();
    for dir in dirs {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "app"))
            .collect();
        files.sort();
        for f in files {
            let name = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let text = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
            out.push(AppTest::parse(&name, &text)?);
        }
    }
    Ok(out)
}

/// A private mount namespace on the reference root, entered with
/// `pivot_root(2)` (so the root is the namespace's own and Chromium may use
/// its user-namespace sandbox, as on a Linux desktop), running `$2`.
const JAIL: &str = r#"set -e
R="$1"
mount --bind "$R" "$R"
mount -t proc proc "$R/proc"
mount --rbind /dev "$R/dev"
umount -l "$R/dev/shm" 2>/dev/null || true
mount -t tmpfs -o mode=1777,size=1g tmpfs "$R/dev/shm"
mount -t sysfs sysfs "$R/sys"
mount -t tmpfs -o mode=1777 tmpfs "$R/tmp"
mount -t tmpfs -o mode=755 tmpfs "$R/run"
cd "$R"; mkdir -p .oldroot; pivot_root . .oldroot; cd /
/usr/bin/umount -l /.oldroot
exec /bin/bash "$2"
"#;

/// Run `tests` on the reference (Linux, as root: `unshare`, `mount`).
fn run_reference(tests: &[AppTest]) -> Result<BTreeMap<String, Outcome>> {
    if std::env::var("POLY_ROLE").is_ok_and(|r| r == "implementer") {
        return Err("implementer agents do not run the reference (CLEAN_ROOM.md)".into());
    }
    let dir = unpack_reference()?;
    let scripts = dir.join("poly-apptests");
    let _ = std::fs::remove_dir_all(&scripts);
    std::fs::create_dir_all(&scripts).map_err(|e| format!("{}: {e}", scripts.display()))?;
    let mut out = BTreeMap::new();
    for t in tests {
        std::fs::write(scripts.join(format!("{}.sh", t.name)), t.script()).map_err(|e| e.to_string())?;
        let run = Command::new("unshare")
            .args(["-m", "--propagation", "private", "bash", "-c", JAIL, "jail"])
            .arg(&dir)
            .arg(format!("/poly-apptests/{}.sh", t.name))
            .output()
            .map_err(|e| format!("unshare: {e}"))?;
        let text = String::from_utf8_lossy(&run.stdout);
        let o = outcome(&text).ok_or_else(|| {
            format!("{}: no outcome on the reference: {}", t.name, String::from_utf8_lossy(&run.stderr).trim())
        })?;
        eprintln!("==> linux {}: status {}, {} bytes of stdout", t.name, o.status, o.stdout.len());
        out.insert(t.name.clone(), o);
    }
    Ok(out)
}

/// Run `tests` on Poly: one boot of Chromium's disk with the tests'
/// scripts in `/poly-apptests/`, each run from the console.
fn run_poly(tests: &[AppTest], release: bool) -> Result<BTreeMap<String, Outcome>> {
    if !cfg!(unix) {
        return Err("the tests are run on Poly through QMP and the serial console: a Unix host".into());
    }
    fetch(false)?;
    let kernel = crate::build_kernel(release)?;
    let syms = Arc::new(Symbols::from_file(&kernel)?);
    let mut contents = collect()?;
    for t in tests {
        contents.tree.file(&format!("poly-apptests/{}.sh", t.name), 0o755, t.script().as_bytes());
    }
    let image = root().join("target/poly-chromium-apptest.img");
    rootdisk::write_image_poly_ext2(&contents.tree, &image)?;
    let initrd = write_initramfs()?;
    let mut m = crate::toolkits::Machine::boot_with_init(
        &kernel,
        &syms,
        &image,
        &initrd,
        "chromium-apptest",
        MEMORY,
        INIT_ARGS,
        CPUS,
    )?;
    m.set_timeouts(Duration::from_secs(900), Duration::from_secs(7200));
    m.desktop_up()?;
    let mut out = BTreeMap::new();
    for t in tests {
        let text = m.run(&format!("bash /poly-apptests/{}.sh", t.name))?;
        match outcome(&text) {
            Some(o) => {
                eprintln!("==> poly {}: status {}, {} bytes of stdout", t.name, o.status, o.stdout.len());
                out.insert(t.name.clone(), o);
            }
            None => eprintln!("==> poly {}: no outcome: {text:?}", t.name),
        }
    }
    let log = root().join("target/poly-chromium-apptest.log");
    let _ = std::fs::write(&log, m.text());
    let _ = m.poweroff();
    Ok(out)
}

/// `cargo xtask chromium apptest [--target linux|poly|both] [--release]
/// [--tests-dir DIR]... [--show]`: run the scenario tests (default
/// [`TESTS_DIR`]) on the reference, on Poly, or both and compare stdout and
/// exit status per test; `--show` prints each test's script.
fn apptest(args: &[String]) -> Result {
    let mut dirs = Vec::new();
    let mut target = "both".to_string();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--tests-dir" => dirs.push(PathBuf::from(it.next().ok_or("--tests-dir DIR")?)),
            "--target" => target = it.next().ok_or("--target linux|poly|both")?.clone(),
            _ => {}
        }
    }
    if dirs.is_empty() {
        dirs.push(root().join(TESTS_DIR));
    }
    let tests = load_tests(&dirs)?;
    if args.iter().any(|a| a == "--show") {
        for t in &tests {
            println!("---- {} ({})\n{}", t.name, t.description, t.script());
        }
        return Ok(());
    }
    let print = args.iter().any(|a| a == "--print");
    let show_all = |runs: &BTreeMap<String, Outcome>| {
        if print {
            for (name, o) in runs {
                println!("---- {name}: status {}\n{}", o.status, String::from_utf8_lossy(&o.stdout));
            }
        }
    };
    let reference = if target != "poly" { Some(run_reference(&tests)?) } else { None };
    if let Some(r) = &reference {
        show_all(r);
    }
    // `linux-twice`: the reference against itself (must be clean, as the
    // pipeline's self-check).
    let poly = match target.as_str() {
        "linux" => None,
        "linux-twice" => Some(run_reference(&tests)?),
        _ => Some(run_poly(&tests, args.iter().any(|a| a == "--release"))?),
    };
    if let Some(p) = poly.as_ref().filter(|_| target == "poly") {
        show_all(p);
    }
    let (Some(reference), Some(poly)) = (reference, poly) else {
        return Ok(());
    };
    let mut failed = Vec::new();
    for t in &tests {
        let (r, p) = (reference.get(&t.name), poly.get(&t.name));
        if r == p {
            eprintln!("==> pass {}", t.name);
        } else {
            let show = |o: Option<&Outcome>| {
                o.map_or("no outcome".into(), |o| {
                    format!("status {} stdout {:?}", o.status, String::from_utf8_lossy(&o.stdout))
                })
            };
            eprintln!("==> FAIL {}: linux {} / poly {}", t.name, show(r), show(p));
            failed.push(t.name.clone());
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("{} of {} tests differ: {}", failed.len(), tests.len(), failed.join(", ")))
    }
}

// ---- the boot test -------------------------------------------------------

/// What the console runs Chromium with: as `poly`, with the sandbox,
/// headless.
fn headless(args: &str, page: &str) -> String {
    format!("/usr/local/bin/poly-chromium --headless=new --log-level=3 {args} file:///{PAGES_DIR}/{page}")
}

/// The boot test of the milestone (see the module documentation). Every
/// check is there and counted; it fails until Poly runs Chromium.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    if !cfg!(unix) {
        eprintln!("==> skipped: the chromium boot test needs a Unix host (QMP)");
        return Ok(());
    }
    let locked = lock()?;
    let absent = missing(&locked, &BUILD_SETS);
    if !absent.is_empty() {
        eprintln!(
            "==> skipped: the chromium boot test: {} locked Debian packages not downloaded (cargo xtask chromium fetch)",
            absent.len()
        );
        return Ok(());
    }
    let initrd = write_initramfs()?;
    let (image, _) = make(TEST_FILE_NAME)?;
    eprintln!("==> boot test: Chromium headless and on the desktop with its sandbox, from its root disk");
    let mut m = crate::toolkits::Machine::boot_with_init(
        kernel,
        syms,
        &image,
        &initrd,
        "chromium-test",
        MEMORY,
        INIT_ARGS,
        CPUS,
    )?;
    m.set_timeouts(Duration::from_secs(600), Duration::from_secs(3600));
    m.desktop_up()?;
    let mut failed = Vec::new();
    // 1. Headless: the DOM after the page's script ran.
    let out = m.run(&headless("--dump-dom", "hello.html"))?;
    if !out.contains("<p id=\"p\">script ran: 42</p>") {
        failed.push(format!("--dump-dom of hello.html: {out:?}"));
    }
    // 2. Headless: a screenshot is written.
    if failed.is_empty() {
        let args = "--screenshot=/home/poly/colours.png --window-size=400,300 --hide-scrollbars";
        let out = m.run(&format!("{} && echo screenshot-\"written\"", headless(args, "colours.html")))?;
        if !out.contains("screenshot-written") {
            failed.push(format!("--screenshot of colours.html: {out:?}"));
        }
    }
    // 3. Windowed with the sandbox, from the launcher: a click turns the
    // page's colour.
    if failed.is_empty()
        && let Err(e) = m.launch_and_click("chromium", "Chromium", INPUT_BEFORE, INPUT_AFTER)
    {
        failed.push(format!("Chromium on the desktop: {e}"));
    }
    let log = root().join("target/poly-chromium-boot.log");
    let _ = std::fs::write(&log, m.text());
    let _ = m.poweroff();
    if failed.is_empty() {
        Ok(())
    } else {
        eprintln!("==> the console's output is in {}", log.display());
        Err(format!("Chromium on Poly: {}", failed.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = "Package: a\nVersion: 1\nDepends: b (>= 2), c | d\nPre-Depends: e:any\n\
                         Filename: pool/main/a/a_1_amd64.deb\nSize: 10\nSHA256: 00\n\n\
                         Package: b\nVersion: 2\nProvides: virt (= 1)\nFilename: pool/main/b/b.deb\n\n\
                         Package: d\nVersion: 1\nDepends: virt\nFilename: pool/main/d/d.deb\n\n\
                         Package: e\nVersion: 1\nDescription: first line\n continued: still description\n\
                         Filename: pool/main/e/e.deb\n";

    fn packages() -> BTreeMap<String, Stanza> {
        parse_index(INDEX, "debian").into_iter().map(|s| (s.package.clone(), s)).collect()
    }

    #[test]
    fn index_stanzas_and_relations() {
        let p = packages();
        assert_eq!(p.len(), 4);
        assert_eq!(p["a"].depends, "e:any, b (>= 2), c | d");
        assert_eq!(p["a"].size, 10);
        assert_eq!(relations(&p["a"].depends), vec![vec!["e"], vec!["b"], vec!["c", "d"]]);
        assert_eq!(relations("x [amd64] (>= 1) | y <!nocheck>"), vec![vec!["x", "y"]]);
    }

    #[test]
    fn closure_takes_the_first_alternative_that_exists_and_providers() {
        let p = packages();
        // c does not exist, so d; d's virt is provided by b, already chosen.
        let got = closure(&["a"], &p).unwrap();
        assert_eq!(got.into_iter().collect::<Vec<_>>(), vec!["a", "b", "d", "e"]);
        assert_eq!(closure(&["virt"], &p).unwrap().into_iter().collect::<Vec<_>>(), vec!["b"]);
        assert!(closure(&["nope"], &p).is_err());
    }

    #[test]
    fn lock_lines() {
        let sum = "ab".repeat(32);
        let text = format!("# comment\nchromium 1-1 debian-security 5 {sum} disk pool/updates/main/c/chromium/c.deb\n");
        let l = parse_lock(&text).unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].file_name(), "c.deb");
        assert!(l[0].on_disk());
        assert!(l[0].url().unwrap().starts_with("https://snapshot.debian.org/archive/debian-security/"));
        assert!(parse_lock(&format!("x 1 debian 5 {sum} disk ../etc/passwd.deb\n")).is_err());
        assert!(parse_lock(&format!("x 1 debian 5 {sum} disk pool/a b.deb\n")).is_err());
        assert!(parse_lock(&format!("x 1 debian 5 {sum} other pool/x.deb\n")).is_err());
        assert!(parse_lock("x 1 debian 5 short disk pool/x.deb\n").is_err());
    }

    #[test]
    fn the_committed_lock_parses_and_has_its_seeds() {
        let l = lock().unwrap();
        for name in DISK_SEEDS {
            assert!(l.iter().any(|p| p.name == *name && p.on_disk()), "{name} not on the disk");
        }
        for name in BOOT_SEEDS.iter().chain(REFERENCE_SEEDS) {
            assert!(l.iter().any(|p| p.name == *name), "{name} not locked");
        }
        let names: BTreeSet<&str> = l.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names.len(), l.len(), "a package locked twice");
    }

    /// A tar header for `name` (ustar, checksum filled in).
    fn header(name: &str, typ: u8, size: usize, mode: u32, link: &str) -> Vec<u8> {
        let mut h = vec![0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..108].copy_from_slice(format!("{mode:07o}\0").as_bytes());
        h[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
        h[136..148].copy_from_slice(format!("{:011o}\0", 1_700_000_000).as_bytes());
        h[156] = typ;
        h[157..157 + link.len()].copy_from_slice(link.as_bytes());
        h[257..263].copy_from_slice(b"ustar\0");
        h
    }

    fn padded(data: &[u8]) -> Vec<u8> {
        let mut v = data.to_vec();
        v.resize(data.len().div_ceil(512) * 512, 0);
        v
    }

    #[test]
    fn tar_entries_of_every_kind() {
        let mut t = Vec::new();
        t.extend(header("./usr/", b'5', 0, 0o755, ""));
        t.extend(header("./usr/lib/chromium/chrome-sandbox", b'0', 3, 0o4755, ""));
        t.extend(padded(b"elf"));
        t.extend(header("./usr/bin/x", b'2', 0, 0o777, "../lib/x"));
        t.extend(header("./usr/bin/y", b'1', 0, 0o644, "./usr/lib/chromium/chrome-sandbox"));
        let long = format!("./usr/share/{}", "n".repeat(120));
        t.extend(header("././@LongLink", b'L', long.len() + 1, 0, ""));
        t.extend(padded(format!("{long}\0").as_bytes()));
        t.extend(header("./usr/share/short", b'0', 0, 0o644, ""));
        t.extend(vec![0u8; 1024]);
        let e = tar_entries(&t).unwrap();
        assert_eq!(e.len(), 5);
        assert_eq!(e[0], TarEntry { path: "usr".into(), mode: 0o755, mtime: 1_700_000_000, kind: TarKind::Dir });
        assert_eq!(e[1].mode, 0o4755);
        assert_eq!(e[1].kind, TarKind::File(b"elf".to_vec()));
        assert_eq!(e[2].kind, TarKind::Symlink("../lib/x".into()));
        assert_eq!(e[3].kind, TarKind::Link("usr/lib/chromium/chrome-sandbox".into()));
        assert_eq!(e[4].path, format!("usr/share/{}", "n".repeat(120)));
    }

    #[test]
    fn ar_members_and_merged_usr() {
        let mut a = b"!<arch>\n".to_vec();
        for (name, data) in [("debian-binary", &b"2.0\n"[..]), ("data.tar", &b"abc"[..])] {
            a.extend(format!("{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n", 0, 0, 0, 100644, data.len()).as_bytes());
            a.extend(data);
            if data.len() % 2 == 1 {
                a.push(b'\n');
            }
        }
        let m = ar_members(&a).unwrap();
        assert_eq!(m, vec![("debian-binary".to_string(), &b"2.0\n"[..]), ("data.tar".to_string(), &b"abc"[..])]);
        assert!(ar_members(b"nope").is_err());
        assert_eq!(merged_usr("bin/bash"), "usr/bin/bash");
        assert_eq!(merged_usr("lib64"), "usr/lib64");
        assert_eq!(merged_usr("binx/y"), "binx/y");
        assert_eq!(merged_usr("usr/bin/env"), "usr/bin/env");
    }

    #[test]
    fn unxz_reads_an_xz_stream() {
        // "hi\n" compressed by `xz -6` (CRC64 check).
        let data: [u8; 60] = [
            0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00, 0x00, 0x04, 0xe6, 0xd6, 0xb4, 0x46, 0x02, 0x00, 0x21, 0x01, 0x16, 0x00,
            0x00, 0x00, 0x74, 0x2f, 0xe5, 0xa3, 0x01, 0x00, 0x02, 0x68, 0x69, 0x0a, 0x00, 0x00, 0xfb, 0x7b, 0xe8, 0xe8,
            0x2a, 0x58, 0x43, 0x4f, 0x00, 0x01, 0x1b, 0x03, 0x0b, 0x2f, 0xb9, 0x10, 0x1f, 0xb6, 0xf3, 0x7d, 0x01, 0x00,
            0x00, 0x00, 0x00, 0x04, 0x59, 0x5a,
        ];
        assert_eq!(unxz(&data).unwrap(), b"hi\n");
        assert!(unxz(b"not xz").is_err());
    }

    #[test]
    fn index_round_trip() {
        let dir = std::env::temp_dir().join(format!("poly-chromium-index-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let entries = vec![
            Unpacked::Dir { path: "usr".into(), mode: 0o755 },
            Unpacked::File { path: "usr/lib/chromium/chrome-sandbox".into(), mode: 0o4755, mtime: 5, blob: 0 },
            Unpacked::Symlink { path: "usr/bin/x y".into(), target: "../lib/x".into() },
        ];
        let path = dir.join("index.txt");
        write_index(&path, "stamp", &entries).unwrap();
        assert_eq!(read_index(&path, "stamp"), Some(entries));
        assert_eq!(read_index(&path, "other"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn base64_and_outcomes() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(b""), "''");
        let o = outcome("bash x\r\nPOLY-APPTEST-OUT 68690a\r\nPOLY-APPTEST-STATUS 0\r\n$ ").unwrap();
        assert_eq!(o, Outcome { stdout: b"hi\n".to_vec(), status: "0".into() });
        assert_eq!(outcome("POLY-APPTEST-STATUS 0\n"), None);
    }

    #[test]
    fn app_tests_parse_and_make_scripts() {
        let t = AppTest::parse(
            "t",
            r#"{"description": "d", "steps": [["chromium", "--dump-dom", "file:///work/a b.html"]],
                "env": ["X=1"], "files": [{"path": "a b.html", "text": "<p>it's</p>"}, {"path": "sub", "dir": true}]}"#,
        )
        .unwrap();
        assert_eq!(t.files.len(), 2);
        let s = t.script();
        assert!(s.contains("env -i 'PATH=/usr/bin:/bin' 'HOME=/work' 'LANG=C' 'TZ=UTC' 'X=1' 'chromium' '--dump-dom' 'file:///work/a b.html'"));
        assert!(s.contains(&format!("echo {} | base64 -d > 'a b.html'", base64(b"<p>it's</p>"))));
        assert!(s.contains("mkdir -p 'sub'\nchmod 755 'sub'"));
        assert!(AppTest::parse("t", r#"{"steps": [["/usr/bin/x"]]}"#).is_err());
        assert!(AppTest::parse("t", r#"{"steps": [["x"]], "other": 1}"#).is_err());
        assert!(AppTest::parse("t", r#"{"steps": [["x"]], "files": [{"path": "../x", "text": ""}]}"#).is_err());
    }

    #[test]
    fn the_visible_tests_parse() {
        let tests = load_tests(&[root().join(TESTS_DIR)]).unwrap();
        assert!(!tests.is_empty());
        assert!(tests.iter().all(|t| t.steps.iter().flatten().any(|a| a.contains("chromium"))));
    }

    #[test]
    fn pages_and_the_launcher_name_what_the_checks_look_for() {
        assert!(PAGES[0].1.contains("6 * 7"));
        assert!(PAGES[1].1.contains("#ffff00"));
        assert!(PAGES[2].1.contains("#b03a2a") && PAGES[2].1.contains("#2a6fb0"));
        assert!(
            desktop_file().contains("Name=Chromium\n")
                && desktop_file().contains("poly-chromium --ozone-platform=wayland")
        );
        assert!(LAUNCH_SCRIPT.contains("setpriv --reuid=1000"));
    }
}
