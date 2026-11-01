//! Synthetic file systems: `/proc` and `/sys`.
//!
//! - [`ProcFs`] is a [`poly_vfs::FileSystem`] whose files the kernel never
//!   stores: their contents are made from a [`ProcSource`] (a trait the
//!   kernel implements over its process table, memory maps, descriptor
//!   tables, mounts, clocks and CPUID) each time they are read, formatted
//!   as the proc(5) family of manual pages describes. `/proc/self`,
//!   `/proc/<pid>/{stat,statm,status,maps,cmdline,environ,comm,exe,cwd,
//!   root,fd/,fdinfo/,mounts,mountinfo,ns/,uid_map,gid_map,setgroups}`,
//!   `/proc/{cpuinfo,meminfo,uptime,
//!   loadavg,version,stat,mounts,filesystems}` and
//!   `/proc/sys/{kernel/{ostype,osrelease,version,hostname,domainname,
//!   pid_max,random/{boot_id,uuid,entropy_avail,poolsize}},fs/nr_open}`.
//! - [`StaticFs`] is a read-only tree of directories, links and files whose
//!   contents a function makes ([`StaticFsBuilder`]): the kernel builds
//!   sysfs with it.
//! - [`format`] holds the text of each file, [`cpuflags`] the names of
//!   CPU features; both are plain functions, tested here.
//!
//! Everything is read-only (`EROFS`) but a process's `uid_map`, `gid_map`
//! and `setgroups`, which the kernel's [`ProcSource::write_id_map`] takes,
//! and `sys/kernel/{hostname,domainname}`
//! ([`ProcSource::write_kernel_name`]); other writes Linux accepts
//! (`comm`) are not supported yet. An open namespace file pins its
//! namespace ([`ProcSource::pin_namespace`]).
//!
//! `/proc/<pid>/ns/<kind>` (namespaces(7)) is a link whose text,
//! `<type>:[<inode number>]`, names a hidden regular file in the same
//! directory: following the link opens that file, and a descriptor on it
//! names the namespace ([`namespace_file`]) for setns(2). (Linux's links
//! are "magic"; Poly's resolve like any other.)

#![no_std]

extern crate alloc;

pub mod cpuflags;
pub mod format;
mod procfs;
mod source;
mod staticfs;

pub use procfs::{NR_OPEN, NS_KINDS, PID_MAX, PID_POS, ProcFs, copy_at, namespace_file, process_dir};
pub use source::{
    CpuFeatures, CpuInfo, CpuTimes, FdInfo, IdMapFile, KernelInfo, KernelName, MapEntry, MapName, MountEntry,
    NANOS_PER_TICK, Owner, ProcSource, ProcState, ProcessInfo, SignalSets, SystemInfo, USER_HZ,
};
pub use staticfs::{Contents, StaticFs, StaticFsBuilder};

#[cfg(test)]
mod tests;
