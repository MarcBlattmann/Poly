//! Blocking opens of FIFOs (fifo(7)): the VFS opens the end at once
//! (`poly_vfs` `fifo.rs`); without `O_NONBLOCK` the opener then waits here
//! for the other side, as a thread that a signal can interrupt. An
//! interrupted open is restarted under `SA_RESTART` (signal(7) lists
//! open(2) of a FIFO among the calls that are), else fails with `EINTR`;
//! the descriptor is closed again either way.

use super::Thread;
use crate::fs::poll::ThreadBlocker;
use crate::fs::{FileType, OpenFlags};
use alloc::sync::Arc;
use poly_personality_linux::errno::Errno;
use poly_vfs::pipe::PipeEnd;

impl Thread<'_> {
    /// After `fd` was opened: if it is a FIFO opened blocking, wait for a
    /// writer (a read end) or a reader (a write end).
    pub(super) fn fifo_open_wait(&self, fd: i32) -> Result<i32, Errno> {
        let files = &self.process.files;
        let Ok(file) = files.file(fd) else { return Ok(fd) };
        if file.kind() != FileType::Fifo || file.flags().contains(OpenFlags::NONBLOCK) {
            return Ok(fd);
        }
        let Some(end) = file.object_as::<PipeEnd>() else { return Ok(fd) };
        drop(file);
        let mut blocker = ThreadBlocker::new(self.process, Arc::clone(&self.flags));
        match end.wait_for_peer(&mut blocker) {
            Ok(()) => Ok(fd),
            Err(_) => {
                let _ = files.close(fd);
                Err(Errno::RESTART_IF_SA_RESTART)
            }
        }
    }
}
