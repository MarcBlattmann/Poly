//! Time (ADR 0044): the structures of the time system calls and the
//! timers of a process. The handlers are in [`crate::sys::time`].
//!
//! - [`abi`]: `timeval`, `itimerval`, `itimerspec`, `sigevent`, `tms`,
//!   `rusage`, `timezone` and their constants.
//! - [`timers`]: interval timers and POSIX timers, fired by the kernel
//!   through [`timers::fire`].

pub mod abi;
pub mod cpuclock;
pub mod timers;

pub use timers::{Fired, Notify, Timer, TimerClock, TimerWakeup, Timers, fire};
