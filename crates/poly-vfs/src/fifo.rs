//! Named pipes (fifo(7)): a FIFO inode made by `mknod` opens as an end of
//! one [`Pipe`] that every open of the inode shares while any end is open
//! (when the last end closes, what was in it is gone). pipe's spec sheet
//! and fifo(7):
//!
//! - `O_RDONLY` opens a read end, `O_WRONLY` a write end, `O_RDWR` both
//!   at once (which never waits: the opener is a reader and a writer).
//! - With `O_NONBLOCK`, opening for reading succeeds without a writer
//!   (reads then return 0 until one opens), opening for writing without a
//!   reader fails with `ENXIO`.
//! - Without it an open waits for the other side to open: `Vfs::open`
//!   itself never blocks; it returns the end and the caller waits with
//!   [`PipeEnd::wait_for_peer`], which the kernel does with its blocker.
//! - `lseek` is `ESPIPE` and the capacity and `PIPE_BUF` rules are a
//!   pipe's (`pipe.rs`).

use crate::Ino;
use crate::pipe::{Pipe, PipeEnd};
use crate::sync::SpinMutex;
use crate::{Errno, Result};
use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};

/// The pipes of the FIFO inodes that are open, by file system and inode.
#[derive(Default)]
pub(crate) struct Fifos {
    pipes: SpinMutex<BTreeMap<(usize, Ino), Weak<Pipe>>>,
}

impl Fifos {
    /// Open the FIFO `(fs, ino)` for reading and/or writing.
    pub(crate) fn open(&self, fs: usize, ino: Ino, read: bool, write: bool, nonblock: bool) -> Result<PipeEnd> {
        let mut pipes = self.pipes.lock();
        let pipe = match pipes.get(&(fs, ino)).and_then(Weak::upgrade) {
            Some(p) => p,
            None => {
                let p = Pipe::fifo();
                pipes.insert((fs, ino), Arc::downgrade(&p));
                p
            }
        };
        // Dead entries of other FIFOs go as they are found.
        pipes.retain(|_, w| w.strong_count() > 0);
        if write && !read && nonblock && pipe.readers() == 0 {
            return Err(Errno::ENXIO);
        }
        Ok(PipeEnd::new(&pipe, read, write))
    }
}
