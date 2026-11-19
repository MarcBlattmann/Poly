//! The logic of the Task Scheduler (ADR 0226), as host-tested code shared by `taskschd.dll` (the
//! COM object model) and `schedsvc.exe` (Poly's `Schedule` service: the task store and the clock).
//!
//! - [`hr`]: the `HRESULT`s the object model returns (`winerror.h`).
//! - [`xml`]: a small XML reader and writer (the task documents only).
//! - [`account`]: the account names a principal accepts and the SIDs written for them.
//! - [`time`]: `DATE`s, the ISO 8601 times and durations of the task schema.
//! - [`def`]: a task definition as data, its defaults and the setters' range checks.
//! - [`write`]: the definition's two XML forms (a definition's `XmlText`, a registered task's `Xml`).
//! - [`parse`]: a task XML document back into a definition, and the registration's validation.
//! - [`path`]: task and folder paths and the errors their forms give.
//! - [`store`]: the folders and tasks of the machine, their states and running instances.
//! - [`sched`]: when a task's triggers fire.
//! - [`persist`]: what the scheduler keeps on disk beside the XML files.
//! - [`service`]: the scheduler answering requests and running its clock.
//! - [`wire`]: the requests and replies between `taskschd.dll` and `schedsvc.exe`.
//!
//! Sources (clean room, CLEAN_ROOM.md): the `taskschd.dll` spec sheets of `spec-win-taskschd` and
//! their visible tests, `specs/windows/taskschd.dll/TASKSCHD.md`, Microsoft Learn ("Task Scheduler
//! 2.0 Interfaces" and each interface's page, "Task Scheduler Schema", "Trigger Types",
//! `TASK_CREATION`, `TASK_LOGON_TYPE`, `TASK_STATE`), `taskschd.h` and `winerror.h`.

#![no_std]

extern crate alloc;

pub mod account;
pub mod def;
pub mod hr;
pub mod parse;
pub mod path;
pub mod persist;
pub mod sched;
pub mod service;
pub mod store;
pub mod time;
pub mod wire;
pub mod write;
pub mod xml;

#[cfg(test)]
mod tests;
