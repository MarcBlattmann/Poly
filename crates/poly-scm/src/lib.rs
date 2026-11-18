//! Windows' service control manager for Poly (ADR 0221).
//!
//! On Windows `services.exe` keeps the service database (`HKLM\SYSTEM\CurrentControlSet\Services`),
//! starts and stops service processes, delivers controls to them and restarts them after a
//! failure; `advapi32` reaches it through RPC. On Poly the manager's state and decisions are this
//! crate ([`manager::Scm`], host-tested): the NT personality keeps one, `advapi32` sends it the
//! messages of [`wire`] through one Poly system call, and `services.exe` (a Poly program in
//! session 0 as LocalSystem) starts the service processes it asks for.
//!
//! Behaviour comes from the spec sheets of the 20 service entries of `advapi32.dll` and their
//! shared facts (`specs/windows/advapi32.dll/SERVICES.md`), constants from `winsvc.h`,
//! `winnt.h` and `winerror.h`.
//!
//! Without the default feature `manager` only [`consts`] and [`wire`] are built (what `advapi32`
//! needs).

#![no_std]

extern crate alloc;

#[cfg(feature = "manager")]
pub mod config;
pub mod consts;
#[cfg(feature = "manager")]
pub mod manager;
pub mod wire;

#[cfg(all(test, feature = "manager"))]
mod tests;

#[cfg(feature = "manager")]
pub use manager::{Caller, Effect, Scm};
