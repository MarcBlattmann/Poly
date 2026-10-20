//! VFS files as the backing of user memory: what `exec` loads a program
//! from and what a private file mapping (`mmap`) reads its pages from.

use super::memory::{IoError, PageSource};
use alloc::sync::Arc;
use poly_vfs::{FileType, OpenFile};

/// An open regular file as a [`PageSource`]: its length is the file's
/// current size, and pages are read through the file system at fault time.
/// The file stays open as long as a mapping of it exists.
#[derive(Debug)]
pub struct FileSource {
    file: Arc<OpenFile>,
}

impl FileSource {
    /// A source over `file`, which must be a regular file; `None` for
    /// anything else (a directory, a device).
    pub fn new(file: Arc<OpenFile>) -> Option<FileSource> {
        (file.kind() == FileType::Regular).then_some(FileSource { file })
    }
}

impl PageSource for FileSource {
    fn len(&self) -> u64 {
        // A file that can no longer be described reads as empty, so its
        // pages fault with SIGBUS, as past the end of a file.
        self.file.stat().map_or(0, |s| s.size)
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, IoError> {
        self.file.read_at(offset, buf).map_err(|_| IoError)
    }
}
