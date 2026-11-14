//! The Applications folder in Files (ADR 0175): `poly_shell::applications`'
//! model as a folder of `poly-files`' file system, so the browser lists,
//! sorts, searches and selects apps as it does files, with its history.
//!
//! [`overlay`] puts the folder over a file system at
//! `poly_shell::applications::DIR`. Reading that folder reads the installed
//! apps again (every visit, a reload, back and forward), so an app that was
//! added or removed shows at the next look. Nothing in it can be written,
//! renamed or removed: it is a list, not a directory. Everything else goes
//! to the file system underneath unchanged.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use poly_files::{Fs, FsError, Kind, Meta, RawEntry};
use poly_shell::App;
use poly_shell::applications::{self as model, Folder};

/// Reads the installed apps (`poly_shell::apps::installed` on the disk; a
/// test's list).
pub type Catalog = Box<dyn Fn() -> Vec<App>>;

/// Reads the desktop file IDs of the apps that can be uninstalled
/// (`poly_pkg::remove::removable` on the disk; a test's list).
pub type Removable = Box<dyn Fn() -> Vec<String>>;

/// The folder as last read, shared with the window (which draws the items'
/// icons and names and opens them).
pub type Shared = Rc<RefCell<Folder>>;

/// A file system with the Applications folder in it.
pub struct WithApplications {
    inner: Box<dyn Fs>,
    catalog: Catalog,
    removable: Removable,
    folder: Shared,
}

impl core::fmt::Debug for WithApplications {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WithApplications").field("apps", &self.folder.borrow().len()).finish_non_exhaustive()
    }
}

/// `fs` with the Applications folder of `catalog`'s apps, and the folder
/// for the window.
pub fn overlay(fs: Box<dyn Fs>, catalog: Catalog, removable: Removable) -> (Box<dyn Fs>, Shared) {
    let folder = Shared::default();
    (Box::new(WithApplications { inner: fs, catalog, removable, folder: Rc::clone(&folder) }), folder)
}

impl WithApplications {
    /// What an item, the folder or a missing item is.
    fn meta(&self, path: &str) -> Result<Meta, FsError> {
        if path == model::DIR {
            return Ok(Meta { kind: Kind::Dir, mode: 0o555, ..Meta::default() });
        }
        match self.folder.borrow().by_path(path) {
            Some(_) => Ok(Meta { kind: Kind::File, mode: 0o444, ..Meta::default() }),
            None => Err(FsError::NotFound),
        }
    }
}

impl Fs for WithApplications {
    fn read_dir(&self, path: &str) -> Result<Vec<RawEntry>, FsError> {
        if path == model::DIR {
            let mut folder = Folder::new((self.catalog)());
            // The apps the installer owns can be uninstalled (ADR 0180).
            folder.mark_removable(&(self.removable)());
            let entries = folder.items().iter().map(|i| RawEntry { name: i.label.clone(), kind: Kind::File }).collect();
            *self.folder.borrow_mut() = folder;
            return Ok(entries);
        }
        if model::contains(path) {
            return Err(FsError::NotADirectory);
        }
        self.inner.read_dir(path)
    }

    fn stat(&self, path: &str) -> Result<Meta, FsError> {
        if model::contains(path) { self.meta(path) } else { self.inner.stat(path) }
    }

    fn lstat(&self, path: &str) -> Result<Meta, FsError> {
        if model::contains(path) { self.meta(path) } else { self.inner.lstat(path) }
    }

    fn read_head(&self, path: &str, max: usize) -> Result<Vec<u8>, FsError> {
        if model::contains(path) { Err(FsError::Unsupported) } else { self.inner.read_head(path, max) }
    }

    fn write(&self, path: &str, data: &[u8]) -> Result<(), FsError> {
        if model::contains(path) { Err(FsError::Unsupported) } else { self.inner.write(path, data) }
    }

    fn create_new(&self, path: &str, data: &[u8]) -> Result<(), FsError> {
        if model::contains(path) { Err(FsError::Unsupported) } else { self.inner.create_new(path, data) }
    }

    fn create_dir(&self, path: &str) -> Result<(), FsError> {
        if model::contains(path) { Err(FsError::Unsupported) } else { self.inner.create_dir(path) }
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), FsError> {
        if model::contains(from) || model::contains(to) {
            Err(FsError::Unsupported)
        } else {
            self.inner.rename(from, to)
        }
    }

    fn remove_file(&self, path: &str) -> Result<(), FsError> {
        if model::contains(path) { Err(FsError::Unsupported) } else { self.inner.remove_file(path) }
    }

    fn remove_dir(&self, path: &str) -> Result<(), FsError> {
        if model::contains(path) { Err(FsError::Unsupported) } else { self.inner.remove_dir(path) }
    }

    fn set_mode(&self, path: &str, mode: u16) -> Result<(), FsError> {
        if model::contains(path) { Err(FsError::Unsupported) } else { self.inner.set_mode(path, mode) }
    }

    fn read_to_string(&self, path: &str) -> Result<String, FsError> {
        if model::contains(path) { Err(FsError::Unsupported) } else { self.inner.read_to_string(path) }
    }

    fn copy_file(&self, from: &str, to: &str) -> Result<(), FsError> {
        if model::contains(from) || model::contains(to) {
            Err(FsError::Unsupported)
        } else {
            self.inner.copy_file(from, to)
        }
    }

    fn exists(&self, path: &str) -> bool {
        if model::contains(path) { self.meta(path).is_ok() } else { self.inner.exists(path) }
    }

    fn is_virtual(&self, path: &str) -> bool {
        path == model::DIR || self.inner.is_virtual(path)
    }
}
