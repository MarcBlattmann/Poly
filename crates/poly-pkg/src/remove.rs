//! What can be uninstalled, and what an uninstall removes (ADR 0180).
//!
//! An app of the launcher is one of four things:
//!
//! - **Poly's own** (`poly:Files`, the shell's windows): never uninstalled;
//! - an **AppImage Poly installed** (a desktop file `poly-appimage-<slug>`
//!   in the user's applications directory that names its image): removed by
//!   deleting its directory and that desktop file ([`crate::appimage`]);
//! - an app of a **Debian package that was installed after the base root was
//!   made**: removed by Debian's own `apt-get remove`, then `apt-get
//!   autoremove` for what only it needed;
//! - a **Windows app** Poly's installer saw being installed (ADR 0230: a
//!   desktop file `poly-windows-<slug>` that names its `Uninstall` key's
//!   `UninstallString`): removed by running that uninstaller;
//! - anything else: the packages the base root is made of (Chromium among
//!   them: the desktop's browser, ADR 0147) and desktop files nobody
//!   installed. Those stay.
//!
//! The base root's own packages are told from later ones by the list the
//! build writes ([`BASE_PACKAGES`]): their names, one per line. dpkg's file
//! lists (`/var/lib/dpkg/info/<package>.list`) say which package owns a
//! desktop file. This module only reads; the commands are [`crate::apt`]'s.

use crate::appimage;
use crate::apt;
use crate::deb::Info;
use crate::dpkgdb;
use crate::installer::{Plan, Removal};
use crate::windows::apps as winapps;
use alloc::collections::BTreeSet;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// The names of the packages the base root was built from, one per line:
/// what is not uninstalled.
pub const BASE_PACKAGES: &str = "/usr/share/poly/base-packages";

/// The disk as this module reads it.
pub trait Disk {
    /// The names in a directory (empty if it is missing).
    fn list(&self, dir: &str) -> Vec<String>;
    /// A file's text, `None` if it is missing or unreadable.
    fn read(&self, path: &str) -> Option<String>;
    /// A file's size in bytes, `None` if it is missing.
    fn size(&self, path: &str) -> Option<u64>;
}

/// The package a file of `/var/lib/dpkg/info` is the list of (`foo.list`,
/// or `foo:amd64.list` for a package that exists for several
/// architectures).
pub fn list_package(file: &str) -> Option<&str> {
    let stem = file.strip_suffix(".list")?;
    let name = stem.split(':').next()?;
    apt::is_package_name(name).then_some(name)
}

fn base_set(disk: &dyn Disk) -> BTreeSet<String> {
    disk.read(BASE_PACKAGES)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Every package's desktop files, as (desktop file ID, package): those of
/// the packages that are in `base` if `base_ones`, else of the others.
fn owned(disk: &dyn Disk, base: &BTreeSet<String>, base_ones: bool) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for file in disk.list(apt::DPKG_INFO) {
        let Some(package) = list_package(&file) else { continue };
        if base.contains(package) != base_ones {
            continue;
        }
        let Some(list) = disk.read(&alloc::format!("{}/{file}", apt::DPKG_INFO)) else { continue };
        for id in dpkgdb::desktop_ids(&list) {
            out.push((id, package.to_string()));
        }
    }
    out
}

/// The desktop file IDs of the apps that can be uninstalled: apps of
/// packages that are not the base root's, and the AppImages Poly installed
/// (`data_home` is the user's data directory). `None` for the base list
/// means a disk that never had one: nothing of it is uninstalled.
pub fn removable(disk: &dyn Disk, data_home: &str) -> Vec<String> {
    let mut out = Vec::new();
    if disk.read(BASE_PACKAGES).is_some() {
        let base = base_set(disk);
        out.extend(owned(disk, &base, false).into_iter().map(|(id, _)| id));
    }
    let dir = appimage::applications_dir(data_home);
    for name in disk.list(&dir) {
        // A Windows app is removable when it registered an uninstaller.
        let windows = || {
            winapps::is_windows_id(&name)
                && disk
                    .read(&alloc::format!("{dir}/{name}"))
                    .is_some_and(|t| winapps::desktop_value(&t, winapps::UNINSTALL_KEY).is_some_and(|v| !v.is_empty()))
        };
        if appimage::slug_of_id(&name).is_some() || windows() {
            out.push(name);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The name a desktop file's text gives its app.
fn app_name(text: &str) -> Option<String> {
    appimage::desktop_keys(text)
        .into_iter()
        .find(|(k, _)| *k == "Name")
        .map(|(_, v)| v.to_string())
        .filter(|n| !n.is_empty())
}

/// What uninstalling the app with desktop file ID `id` removes, or why it
/// cannot be uninstalled.
pub fn plan(id: &str, data_home: &str, disk: &dyn Disk) -> Result<Plan, String> {
    if let Some(name) = id.strip_prefix("poly:") {
        return Err(alloc::format!("{name} is part of Poly and cannot be uninstalled."));
    }
    if let Some(slug) = appimage::slug_of_id(id) {
        let layout = appimage::layout(data_home, slug);
        let text = disk.read(&layout.desktop).ok_or("This app is not installed.")?;
        if appimage::image_of_desktop(&text).as_deref() != Some(layout.image.as_str()) {
            return Err("This app was not installed by Poly's installer, so Poly does not remove it.".to_string());
        }
        let app = app_name(&text).unwrap_or_else(|| slug.to_string());
        let size = disk.size(&layout.image).unwrap_or(0);
        return Ok(Plan {
            apps: alloc::vec![(layout.id.clone(), app.clone())],
            app,
            removal: Removal::Image { slug: slug.to_string(), size },
            package: None,
        });
    }
    if winapps::is_windows_id(id) {
        return windows_plan(id, data_home, disk);
    }
    if !id.ends_with(".desktop") || id.contains('/') {
        return Err("This is not an app Poly knows how to uninstall.".to_string());
    }
    let base = base_set(disk);
    let package = match owned(disk, &base, false).into_iter().find(|(i, _)| i == id) {
        Some((_, package)) => package,
        None => {
            return Err(match owned(disk, &base, true).into_iter().find(|(i, _)| i == id) {
                Some((_, package)) => alloc::format!(
                    "{package} is part of Poly's base system (the desktop and its browser come from it), so it stays."
                ),
                None => "This app did not come from a package Poly installed, so Poly does not remove it.".to_string(),
            });
        }
    };
    let status = disk.read(apt::DPKG_STATUS).unwrap_or_default();
    let info: Info =
        dpkgdb::installed_info(&status, &package).ok_or_else(|| alloc::format!("{package} is not installed."))?;
    // What goes from the launcher: every desktop file the package has.
    let list = disk
        .list(apt::DPKG_INFO)
        .into_iter()
        .filter(|f| list_package(f) == Some(package.as_str()))
        .find_map(|f| disk.read(&alloc::format!("{}/{f}", apt::DPKG_INFO)))
        .unwrap_or_default();
    let apps: Vec<(String, String)> = dpkgdb::desktop_ids(&list)
        .into_iter()
        .map(|i| {
            let name = disk
                .read(&alloc::format!("/usr/share/applications/{i}"))
                .and_then(|t| app_name(&t))
                .unwrap_or_else(|| i.clone());
            (i, name)
        })
        .collect();
    let app = apps.iter().find(|(i, _)| i == id).map_or_else(|| package.clone(), |(_, n)| n.clone());
    Ok(Plan { app, removal: Removal::Package, package: Some(info), apps })
}

/// The disk as [`winapps`] reads it.
struct AsFiles<'d>(&'d dyn Disk);

impl winapps::Files for AsFiles<'_> {
    fn list(&self, dir: &str) -> Vec<String> {
        self.0.list(dir)
    }

    fn is_dir(&self, path: &str) -> bool {
        !self.0.list(path).is_empty()
    }

    fn exists(&self, path: &str) -> bool {
        self.0.size(path).is_some() || self.is_dir(path)
    }
}

/// What uninstalling the Windows app `id` runs (ADR 0230): its
/// `UninstallString`, and the launcher entries of the same `Uninstall` key.
fn windows_plan(id: &str, data_home: &str, disk: &dyn Disk) -> Result<Plan, String> {
    let dir = appimage::applications_dir(data_home);
    let text = disk.read(&alloc::format!("{dir}/{id}")).ok_or("This app is not installed.")?;
    let app = app_name(&text).unwrap_or_else(|| id.to_string());
    let uninstall =
        winapps::desktop_value(&text, winapps::UNINSTALL_KEY).filter(|v| !v.is_empty()).ok_or_else(|| {
            alloc::format!("{app} did not register an uninstaller with Windows, so Poly cannot remove it.")
        })?;
    let command = winapps::uninstall_argv(uninstall, &AsFiles(disk))
        .ok_or_else(|| alloc::format!("{app}'s uninstaller is not there ({uninstall})."))?;
    let key = winapps::desktop_value(&text, winapps::REGISTRY_KEY).map(str::to_string);
    let mut apps = alloc::vec![(id.to_string(), app.clone())];
    if let Some(key) = &key {
        for other in disk.list(&dir) {
            if other == id || !winapps::is_windows_id(&other) {
                continue;
            }
            let Some(t) = disk.read(&alloc::format!("{dir}/{other}")) else { continue };
            if winapps::desktop_value(&t, winapps::REGISTRY_KEY) == Some(key.as_str()) {
                apps.push((other.clone(), app_name(&t).unwrap_or(other)));
            }
        }
    }
    let value = |k: &str| winapps::desktop_value(&text, k).unwrap_or("").to_string();
    Ok(Plan {
        app,
        removal: Removal::Windows {
            command,
            publisher: value(winapps::PUBLISHER_KEY),
            version: value(winapps::VERSION_KEY),
        },
        package: None,
        apps,
    })
}

#[cfg(test)]
mod tests;
