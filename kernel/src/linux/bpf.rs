//! bpf(2) in the kernel (the personality's `bpf` module does the work,
//! ADR 0135): the file object behind a map's or a program's descriptor.

use super::Thread;
use super::services::vfs_errno;
use crate::fs::{self, OpenFile, OpenFlags};
use alloc::string::String;
use alloc::sync::Arc;
use poly_personality_linux::bpf::Object;
use poly_personality_linux::errno::Errno;
use poly_vfs::{FileObject, PollEvents, WaitQueue};

/// What a map's or program's descriptor is open on.
#[derive(Debug)]
pub struct BpfFile {
    object: Object,
}

impl FileObject for BpfFile {
    fn read(&self, _buf: &mut [u8]) -> poly_vfs::Result<usize> {
        Err(poly_vfs::Errno::EINVAL)
    }

    fn write(&self, _buf: &[u8]) -> poly_vfs::Result<usize> {
        Err(poly_vfs::Errno::EINVAL)
    }

    fn poll(&self) -> PollEvents {
        PollEvents::NONE
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        None
    }

    fn anon_name(&self) -> Option<&'static str> {
        Some(self.object.anon_name())
    }

    fn fdinfo(&self) -> String {
        self.object.fdinfo()
    }
}

/// `LinuxKernel::bpf_install`.
pub fn install(k: &Thread<'_>, object: Object, cloexec: bool) -> Result<i32, Errno> {
    let files = &k.process().files;
    let node = fs::vfs().anon_inode(0o600, &files.cred()).map_err(vfs_errno)?;
    let file = Arc::new(OpenFile::on_object(node, Arc::new(BpfFile { object }), OpenFlags::RDWR));
    files.install(file, cloexec).map_err(vfs_errno)
}

/// `LinuxKernel::bpf_object`.
pub fn object(k: &Thread<'_>, fd: i32) -> Result<Option<Object>, Errno> {
    let file = k.process().files.file(fd).map_err(vfs_errno)?;
    Ok(file.object_as::<BpfFile>().map(|b| b.object.clone()))
}
