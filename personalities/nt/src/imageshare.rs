//! Clean image pages shared by every process that maps the same image file
//! (ADR 0188).
//!
//! Every Chrome process maps `chrome.dll` (310 MB) and the system DLLs. Each
//! process makes its own image section of the file, and a page a view
//! touched used to be a private copy in each process. Views of an image are
//! copy on write (Learn, "Memory Protection Constants", `PAGE_WRITECOPY`: a
//! write to such a page makes a private copy of it for the process), so a
//! page nobody wrote can be one frame in every process that maps the image,
//! and is:
//!
//! - An image read from a file is named by the file's [`FileIdentity`]
//!   (device, inode, size and modification time: a new file, or a changed
//!   one, is a new identity) and the base its bytes are relocated for: 0 for
//!   the image as laid out at its preferred base, else the base of a view
//!   the kernel relocated. The same file relocated to the same base is the
//!   same bytes, so those views share too: in Chrome's processes
//!   `chrome_elf.dll` takes the usual DLL base first and the 310 MB
//!   `chrome.dll` is relocated, to the same address in each of them.
//! - The [`Registry`] maps an identity to the one object every section of
//!   it shares (the kernel's table of its pages); it holds it weakly, so the
//!   object goes with the last section that uses it.
//! - [`SharedPages`] maps a page's offset to its frame, weakly: a frame lives
//!   while some process maps it and goes with the last mapping, as a private
//!   copy did; a process that writes a page gets a copy of its own (or takes
//!   the frame over if nobody else maps it), so the shared frame never
//!   changes.
//!
//! Both are plain data: the kernel puts them behind its locks, and fills a
//! missing page with no lock held, then calls [`SharedPages::insert`], where
//! a page another process made meanwhile wins.

use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};

/// What names an image file's contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileIdentity {
    pub dev: u64,
    pub ino: u64,
    pub size: u64,
    pub mtime_sec: i64,
    pub mtime_nsec: u32,
    /// The base the bytes are relocated for: 0 for the image as laid out at its preferred base.
    pub base: u64,
}

/// How many insertions [`SharedPages`] and [`Registry`] allow between two
/// sweeps of entries whose object is gone.
const SWEEP_EVERY: usize = 1024;

/// The pages of one image that some process maps, by offset.
#[derive(Debug)]
pub struct SharedPages<T> {
    pages: BTreeMap<u64, Weak<T>>,
    inserts: usize,
}

impl<T> Default for SharedPages<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> SharedPages<T> {
    pub const fn new() -> SharedPages<T> {
        SharedPages { pages: BTreeMap::new(), inserts: 0 }
    }

    /// The page at `offset`, if a process maps it.
    pub fn get(&self, offset: u64) -> Option<Arc<T>> {
        self.pages.get(&offset).and_then(Weak::upgrade)
    }

    /// Keep `page` as the page at `offset`, unless a live one is there
    /// already (made by another process meanwhile): returns the one to map.
    pub fn insert(&mut self, offset: u64, page: Arc<T>) -> Arc<T> {
        if let Some(live) = self.get(offset) {
            return live;
        }
        self.inserts += 1;
        if self.inserts >= SWEEP_EVERY {
            self.inserts = 0;
            self.pages.retain(|_, w| w.strong_count() > 0);
        }
        self.pages.insert(offset, Arc::downgrade(&page));
        page
    }

    /// Pages some process maps now.
    pub fn live(&self) -> usize {
        self.pages.values().filter(|w| w.strong_count() > 0).count()
    }

    /// Entries kept, live or not (bounded by the sweep).
    pub fn entries(&self) -> usize {
        self.pages.len()
    }
}

/// The shared object of each image file some section uses.
#[derive(Debug)]
pub struct Registry<V> {
    images: BTreeMap<FileIdentity, Weak<V>>,
}

impl<V> Default for Registry<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V> Registry<V> {
    pub const fn new() -> Registry<V> {
        Registry { images: BTreeMap::new() }
    }

    /// The object for `key`, made with `make` if no section uses one now.
    pub fn get_or_insert_with(&mut self, key: FileIdentity, make: impl FnOnce() -> V) -> Arc<V> {
        if let Some(live) = self.images.get(&key).and_then(Weak::upgrade) {
            return live;
        }
        // Few images: sweep the dead ones whenever a new one comes.
        self.images.retain(|_, w| w.strong_count() > 0);
        let new = Arc::new(make());
        self.images.insert(key, Arc::downgrade(&new));
        new
    }

    /// Images some section uses now.
    pub fn live(&self) -> usize {
        self.images.values().filter(|w| w.strong_count() > 0).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(ino: u64) -> FileIdentity {
        FileIdentity { dev: 1, ino, size: 0x1000, mtime_sec: 5, mtime_nsec: 0, base: 0 }
    }

    #[test]
    fn a_page_is_shared_while_mapped_and_goes_with_the_last_mapping() {
        let mut pages: SharedPages<u32> = SharedPages::new();
        assert!(pages.get(0x1000).is_none());
        let a = pages.insert(0x1000, Arc::new(7));
        // A second process finds the same page.
        let b = pages.get(0x1000).expect("mapped");
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(pages.live(), 1);
        drop(a);
        assert!(pages.get(0x1000).is_some(), "still mapped by one");
        drop(b);
        assert!(pages.get(0x1000).is_none(), "gone with the last mapping");
        assert_eq!(pages.live(), 0);
    }

    #[test]
    fn a_page_made_meanwhile_wins() {
        let mut pages: SharedPages<u32> = SharedPages::new();
        let first = pages.insert(0, Arc::new(1));
        // Another fault filled the same page with no lock held.
        let second = pages.insert(0, Arc::new(2));
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(*second, 1);
        // Once nobody maps it, a new page takes its place.
        drop((first, second));
        assert_eq!(*pages.insert(0, Arc::new(3)), 3);
    }

    #[test]
    fn a_page_taken_over_by_its_last_mapping_is_not_found_again() {
        // The last user of a frame that writes it makes the frame its own
        // (`Arc::try_unwrap`): the table must not hand it out afterwards.
        let mut pages: SharedPages<u32> = SharedPages::new();
        let only = pages.insert(0x2000, Arc::new(9));
        let owned = Arc::try_unwrap(only).expect("the last user");
        assert_eq!(owned, 9);
        assert!(pages.get(0x2000).is_none());
    }

    #[test]
    fn dead_entries_are_swept() {
        let mut pages: SharedPages<u64> = SharedPages::new();
        for i in 0..(SWEEP_EVERY as u64 * 3) {
            drop(pages.insert(i * 0x1000, Arc::new(i)));
        }
        assert!(pages.entries() <= SWEEP_EVERY, "{} entries kept", pages.entries());
        assert_eq!(pages.live(), 0);
    }

    #[test]
    fn the_registry_hands_out_one_object_per_file_while_used() {
        let mut reg: Registry<SharedPages<u32>> = Registry::new();
        let a = reg.get_or_insert_with(id(1), SharedPages::new);
        let b = reg.get_or_insert_with(id(1), SharedPages::new);
        assert!(Arc::ptr_eq(&a, &b));
        let other = reg.get_or_insert_with(id(2), SharedPages::new);
        assert!(!Arc::ptr_eq(&a, &other));
        // A changed file (another modification time) is another image.
        let changed = reg.get_or_insert_with(FileIdentity { mtime_sec: 6, ..id(1) }, SharedPages::new);
        assert!(!Arc::ptr_eq(&a, &changed));
        assert_eq!(reg.live(), 3);
        drop((a, b));
        assert_eq!(reg.live(), 2);
        // Made again once every section of it is gone.
        let again = reg.get_or_insert_with(id(1), SharedPages::new);
        assert_eq!(again.entries(), 0);
        assert_eq!(reg.live(), 3);
    }
}
