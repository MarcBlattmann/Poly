//! acct(2) in the kernel (ADR 0135): the file accounting records are
//! appended to, kept open by the pid namespace's accounting.

use super::Thread;
use super::services::vfs_errno;
use crate::fs;
use alloc::sync::Arc;
use poly_personality_linux::acct::Sink;
use poly_personality_linux::errno::Errno;
use poly_vfs::{At, OpenFile, OpenFlags};

/// An accounting file open for appending.
struct File {
    file: Arc<OpenFile>,
}

impl Sink for File {
    fn append(&self, record: &[u8]) {
        // A full disk or a vanished file loses the record.
        let _ = self.file.write(record);
    }
}

/// `LinuxKernel::acct_open`.
pub fn open(k: &Thread<'_>, path: &[u8]) -> Result<Arc<dyn Sink>, Errno> {
    let files = &k.process().files;
    let flags = OpenFlags::WRONLY | OpenFlags::APPEND;
    let file = fs::vfs().open(&files.context(), &files.cred(), At::Cwd, path, flags, 0).map_err(vfs_errno)?;
    Ok(Arc::new(File { file }))
}
