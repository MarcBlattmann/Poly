//! uprobe(2) and uretprobe(2): system calls only the kernel's own probe
//! trampolines may make (ADR 0134). Poly installs no probes, so every call
//! comes from somewhere else: uprobe fails with `ENXIO`; uretprobe forces
//! `SIGILL` (`SI_KERNEL`) on the caller and returns -1, as the spec
//! sheets record.

use crate::abi::SigInfo;
use crate::abi::signal::{SI_KERNEL, SIGILL};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::table::{Handler, nr};

pub const CALLS: &[(u64, Handler)] = &[(nr::UPROBE, uprobe), (nr::URETPROBE, uretprobe)];

/// uprobe(): `ENXIO` from anywhere but the uprobe trampoline.
pub fn uprobe(_: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    Err(Errno::ENXIO)
}

/// uretprobe(): `SIGILL` forced on the caller (unblocked, an ignored
/// action reset), then -1 in `rax` (the bits of `-EPERM`) if a handler
/// returns.
pub fn uretprobe(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    crate::signal::force_signal(k, SigInfo::new(SIGILL, SI_KERNEL));
    Err(Errno::EPERM)
}
