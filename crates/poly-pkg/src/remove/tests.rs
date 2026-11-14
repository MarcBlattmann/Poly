use super::*;
use alloc::collections::BTreeMap;

#[derive(Default)]
struct Mem {
    files: BTreeMap<String, String>,
    sizes: BTreeMap<String, u64>,
}

impl Mem {
    fn put(&mut self, path: &str, text: &str) {
        self.files.insert(path.to_string(), text.to_string());
    }
}

impl Disk for Mem {
    fn list(&self, dir: &str) -> Vec<String> {
        let prefix = alloc::format!("{}/", dir.trim_end_matches('/'));
        self.files
            .keys()
            .filter_map(|p| p.strip_prefix(&prefix))
            .filter(|rest| !rest.contains('/'))
            .map(ToString::to_string)
            .collect()
    }

    fn read(&self, path: &str) -> Option<String> {
        self.files.get(path).cloned()
    }

    fn size(&self, path: &str) -> Option<u64> {
        self.sizes.get(path).copied()
    }
}

const STATUS: &str = "Package: galculator\nStatus: install ok installed\nVersion: 2.1.4-2\nArchitecture: amd64\n\
Maintainer: Debian QA Group <packages@qa.debian.org>\nInstalled-Size: 1297\nDescription: scientific calculator\n A GTK 3 calculator.\n\n\
Package: chromium\nStatus: install ok installed\nVersion: 140.0\nArchitecture: amd64\nDescription: web browser\n\n\
Package: oldtool\nStatus: deinstall ok config-files\nVersion: 1\nArchitecture: amd64\nDescription: gone\n";

fn disk() -> Mem {
    let mut d = Mem::default();
    d.put(BASE_PACKAGES, "chromium\nlibc6\n\n");
    d.put(apt::DPKG_STATUS, STATUS);
    d.put(
        "/var/lib/dpkg/info/galculator.list",
        "/.\n/usr/bin/galculator\n/usr/share/applications/galculator.desktop\n",
    );
    d.put("/var/lib/dpkg/info/chromium.list", "/usr/share/applications/chromium.desktop\n");
    d.put("/var/lib/dpkg/info/libc6:amd64.list", "/usr/lib/x86_64-linux-gnu/libc.so.6\n");
    d.put("/var/lib/dpkg/info/galculator.md5sums", "x  usr/bin/galculator\n");
    d.put(
        "/usr/share/applications/galculator.desktop",
        "[Desktop Entry]\nType=Application\nName=Galculator\nExec=galculator\n",
    );
    d.put(
        "/usr/share/applications/chromium.desktop",
        "[Desktop Entry]\nType=Application\nName=Chromium\nExec=chromium\n",
    );
    d
}

fn with_image(d: &mut Mem) {
    let l = appimage::layout("/home/poly/.local/share", "hello-pixels");
    d.put(
        &l.desktop,
        &alloc::format!(
            "[Desktop Entry]\nType=Application\nName=Hello Pixels\nExec={0}\nX-Poly-AppImage={0}\n",
            l.image
        ),
    );
    d.sizes.insert(l.image, 5_000_000);
}

#[test]
fn list_files_name_their_package() {
    assert_eq!(list_package("galculator.list"), Some("galculator"));
    assert_eq!(list_package("libc6:amd64.list"), Some("libc6"));
    assert_eq!(list_package("galculator.md5sums"), None);
    assert_eq!(list_package(".list"), None);
    assert_eq!(list_package("Bad Name.list"), None);
}

#[test]
fn only_apps_installed_after_the_base_root_can_be_uninstalled() {
    let mut d = disk();
    assert_eq!(removable(&d, "/home/poly/.local/share"), ["galculator.desktop"]);
    with_image(&mut d);
    assert_eq!(removable(&d, "/home/poly/.local/share"), ["galculator.desktop", "poly-appimage-hello-pixels.desktop"]);
    // A disk that never had the base list removes nothing of dpkg's.
    d.files.remove(BASE_PACKAGES);
    assert_eq!(removable(&d, "/home/poly/.local/share"), ["poly-appimage-hello-pixels.desktop"]);
    // A desktop file of the user that is no AppImage of ours is not offered.
    d.put("/home/poly/.local/share/applications/mine.desktop", "[Desktop Entry]\n");
    d.put("/home/poly/.local/share/applications/poly-appimage-..x.desktop", "[Desktop Entry]\n");
    assert_eq!(removable(&d, "/home/poly/.local/share"), ["poly-appimage-hello-pixels.desktop"]);
}

#[test]
fn a_package_that_came_later_is_planned_with_what_goes() {
    let d = disk();
    let plan = plan("galculator.desktop", "/home/poly/.local/share", &d).unwrap();
    assert_eq!(plan.app, "Galculator");
    assert_eq!(plan.removal, Removal::Package);
    assert_eq!(plan.apps, [("galculator.desktop".to_string(), "Galculator".to_string())]);
    let info = plan.package.unwrap();
    assert_eq!(
        (info.name.as_str(), info.version.as_str(), info.summary.as_str()),
        ("galculator", "2.1.4-2", "scientific calculator")
    );
    assert_eq!(info.installed_kib, Some(1297));
}

#[test]
fn the_base_root_and_poly_itself_stay() {
    let d = disk();
    let why = plan("chromium.desktop", "/h", &d).unwrap_err();
    assert!(why.starts_with("chromium is part of Poly's base system"), "{why}");
    assert_eq!(plan("poly:Files", "/h", &d).unwrap_err(), "Files is part of Poly and cannot be uninstalled.");
    assert!(plan("nothing.desktop", "/h", &d).unwrap_err().contains("did not come from a package"));
    for bad in ["../etc/passwd", "a/b.desktop", "x", ""] {
        assert!(plan(bad, "/h", &d).is_err(), "{bad}");
    }
    // A package dpkg no longer has installed.
    let mut d = disk();
    d.put("/var/lib/dpkg/info/oldtool.list", "/usr/share/applications/old.desktop\n");
    assert_eq!(plan("old.desktop", "/h", &d).unwrap_err(), "oldtool is not installed.");
}

#[test]
fn an_appimage_is_planned_from_its_desktop_file() {
    let mut d = disk();
    with_image(&mut d);
    let plan = plan("poly-appimage-hello-pixels.desktop", "/home/poly/.local/share", &d).unwrap();
    assert_eq!(plan.app, "Hello Pixels");
    assert_eq!(plan.removal, Removal::Image { slug: "hello-pixels".into(), size: 5_000_000 });
    assert_eq!(plan.package, None);
    // Not there, or a desktop file that does not name its image.
    assert_eq!(
        super::plan("poly-appimage-gone.desktop", "/home/poly/.local/share", &d).unwrap_err(),
        "This app is not installed."
    );
    let l = appimage::layout("/home/poly/.local/share", "other");
    d.put(&l.desktop, "[Desktop Entry]\nName=Other\nExec=x\n");
    assert!(super::plan(&l.id, "/home/poly/.local/share", &d).unwrap_err().contains("not installed by Poly"));
}
