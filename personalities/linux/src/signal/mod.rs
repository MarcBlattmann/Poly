//! Signals: generation, pending queues, delivery to handlers, signal
//! frames, rt_sigreturn, alternate stacks, and restarting interrupted
//! system calls (ADR 0031).
//!
//! Behaviour follows signal(7), sigaction(2), sigaltstack(2), sigreturn(2),
//! kill(2), tgkill(2), sigprocmask(2), sigpending(2), sigsuspend(2),
//! sigwaitinfo(2) and POSIX; constants and layouts come from the UAPI
//! headers. Points the documentation leaves open are marked "to be
//! confirmed by the diff runner" where they are decided.
//!
//! ```text
//!  kill / tgkill / fault          LinuxProcess (per process, kernel-locked)
//!  ───────────────────►  send ──► shared queue / thread queue ──► notify_signal(tid)
//!                                                                   │ ends block()
//!  kernel, before every return to user mode:                        ▼
//!  exit_to_user ──► dequeue ──► ignore / stop_process / kill_process / frame on the user stack
//!  rt_sigreturn ──► registers, mask, FPU state, alternate stack back from the frame
//! ```
//!
//! - [`action`]: default actions and dispositions.
//! - [`pending`]: queues and delivery order.
//! - [`state`]: [`LinuxProcess`] / [`LinuxThread`], what the kernel stores.
//! - [`altstack`]: sigaltstack(2) rules.
//! - [`frame`]: the x86-64 frame layout.
//! - [`restart`]: `SA_RESTART` / `EINTR` / restart_syscall(2).
//! - [`deliver`]: sending, [`deliver::exit_to_user`], rt_sigreturn.

pub mod action;
pub mod altstack;
pub mod deliver;
pub mod frame;
pub mod pending;
pub mod restart;
pub mod state;

pub use altstack::AltStack;
pub use deliver::{ExitToUser, exit_to_user, force_signal};
pub use state::{Dest, LinuxProcess, LinuxThread, Sent};
