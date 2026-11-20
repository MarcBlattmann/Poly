//! The Mach core of Poly's XNU personality (ADR 0238): port name spaces,
//! rights, port sets, message queues and `mach_msg` semantics, task and
//! thread self ports, semaphores, and the wait tables behind `ulock` and
//! `psynch`.
//!
//! Nothing here touches hardware or blocks. The kernel keeps one [`Ipc`],
//! one [`UlockTable`] and one [`PsynchTable`] behind a lock; the XNU
//! personality (`personalities/xnu`) decodes the traps, copies user memory,
//! and blocks and wakes threads through the kernel. A call that has to wait
//! queues the calling thread here and the personality blocks it; a waker
//! leaves a result and returns the threads to unblock (see [`waitq`]).
//!
//! Behaviour follows the spec sheets in `specs/macos/mach/` and
//! `specs/macos/bsd/` and the public Mach documentation; constants come from
//! the public SDK headers ([`consts`]).

#![no_std]

extern crate alloc;

pub mod consts;
pub mod exc;
pub mod ipc;
pub mod psynch;
pub mod ulock;
pub mod waitq;

pub use ipc::{
    Exception, Ipc, KObject, NoThreads, Options, Peeked, PortId, RcvKey, Received, SendError, SenderInfo, Service,
    SetId, Signal, Target, TaskId, ThreadServer, WakeReason,
};
pub use psynch::PsynchTable;
pub use ulock::UlockTable;
pub use waitq::{ThreadId, WaitQueue};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_ports;
