//! System call handlers, grouped by subsystem. Each handler decodes its
//! arguments from [`Args`](crate::args::Args), validates them as the man
//! page describes and calls the [`LinuxKernel`](crate::kernel::LinuxKernel)
//! for the work. They are registered by number in [`crate::table`].

pub mod fd;
pub mod futex;
pub mod io;
pub mod limits;
pub mod mem;
pub mod path;
pub mod process;
pub mod signal;
pub mod system;
pub mod task;
