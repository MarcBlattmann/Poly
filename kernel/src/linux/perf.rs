//! perf_event_open(2) in the kernel (the personality's `perf` module does
//! the work, ADR 0135): what a counter counts from (a process's CPU time
//! and page faults, which the kernel accounts anyway), the file object
//! behind a counter's descriptor, and its mmap(2) ring, which is io_uring's
//! kind of shared ring memory.

use super::Thread;
use super::services::vfs_errno;
use crate::fs::{self, OpenFile, OpenFlags};
use crate::process::Process;
use crate::user::memory::PageSource;
use alloc::sync::Arc;
use poly_personality_linux::errno::Errno;
use poly_personality_linux::perf::{Counter, PerfSource, sw};
use poly_vfs::{FileObject, PollEvents, WaitQueue};

/// A process's accounting, kept alive with it (a counter on another
/// process stays readable after that one was reaped).
#[derive(Debug)]
struct TaskSource {
    process: Arc<Process>,
}

impl PerfSource for TaskSource {
    fn total(&self, config: u64) -> u64 {
        match config {
            sw::CPU_CLOCK | sw::TASK_CLOCK => {
                let t = crate::cputime::process(&self.process);
                t.user_ns + t.system_ns
            }
            sw::PAGE_FAULTS | sw::PAGE_FAULTS_MIN => self.process.cpu.memory_counts().0,
            _ => 0,
        }
    }

    fn now_ns(&self) -> u64 {
        crate::time::now_ns()
    }
}

/// What a counter's descriptor is open on.
pub struct PerfFile {
    counter: Arc<Counter>,
}

impl core::fmt::Debug for PerfFile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PerfFile").field("counter", &self.counter).finish()
    }
}

impl FileObject for PerfFile {
    fn read(&self, buf: &mut [u8]) -> poly_vfs::Result<usize> {
        let bytes = self.counter.read(buf.len()).map_err(|_| poly_vfs::Errno::ENOSPC)?;
        buf[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len())
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
        Some("[perf_event]")
    }
}

/// `LinuxKernel::perf_source`.
pub fn source(k: &Thread<'_>, pid: i32) -> Result<Arc<dyn PerfSource>, Errno> {
    let process =
        if pid == 0 { crate::user::current_process().ok_or(Errno::ESRCH)? } else { super::procaccess::process(pid)? };
    let _ = k;
    Ok(Arc::new(TaskSource { process }))
}

/// `LinuxKernel::perf_install`.
pub fn install(k: &Thread<'_>, counter: Arc<Counter>, cloexec: bool) -> Result<i32, Errno> {
    let files = &k.process().files;
    let node = fs::vfs().anon_inode(0o600, &files.cred()).map_err(vfs_errno)?;
    let file = Arc::new(OpenFile::on_object(node, Arc::new(PerfFile { counter }), OpenFlags::RDWR));
    files.install(file, cloexec).map_err(vfs_errno)
}

/// `LinuxKernel::perf_counter`.
pub fn counter(k: &Thread<'_>, fd: i32) -> Result<Option<Arc<Counter>>, Errno> {
    let file = k.process().files.file(fd).map_err(vfs_errno)?;
    Ok(file.object_as::<PerfFile>().map(|p| Arc::clone(&p.counter)))
}

/// mmap(2) of a counter's descriptor (`None` if `file` is none): its ring
/// memory, by the personality's rules (`EINVAL`).
pub fn mapping(file: &OpenFile, offset: u64, len: u64, shared: bool) -> Option<Result<Arc<dyn PageSource>, Errno>> {
    let perf = file.object_as::<PerfFile>()?;
    let ring = perf.counter.map(len, offset, shared, super::uring::ring_mapped, |len| Ok(super::uring::ring(len)));
    Some(ring.and_then(|r| super::uring::ring_pages(&r).ok_or(Errno::EINVAL)))
}
