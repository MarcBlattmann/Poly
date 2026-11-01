//! System call handlers, grouped by subsystem. Each handler decodes its
//! arguments from [`Args`](crate::args::Args), validates them as the man
//! page describes and calls the [`LinuxKernel`](crate::kernel::LinuxKernel)
//! for the work. They are registered by number in [`crate::table`].

pub mod admin;
pub mod cred;
pub mod epoll;
pub mod eventfd;
pub mod fadvise;
pub mod falloc;
pub mod fd;
pub mod fdioctl;
pub mod fileioctl;
pub mod fsize;
pub mod futex;
pub mod futex2;
pub mod inotify;
pub mod io;
pub mod ioprio;
pub mod limits;
pub mod madvise;
pub mod mem;
pub mod membarrier;
pub mod memfd;
pub mod mlock;
pub mod msg;
pub mod ns;
pub mod path;
pub mod perm;
pub mod personality;
pub mod pidfd;
pub mod pipe;
pub mod pkey;
pub mod poll;
pub mod prctl;
pub mod procaccess;
pub mod process;
pub mod rseq;
pub mod rwv;
pub mod sched;
pub mod seccomp;
pub mod sem;
pub mod sendfile;
pub mod shm;
pub mod signal;
pub mod signalfd;
pub mod socket;
pub mod splice;
pub mod subreaper;
pub mod system;
pub mod task;
pub mod time;
pub mod timerfd;
pub mod utimes;
pub mod xattr;
