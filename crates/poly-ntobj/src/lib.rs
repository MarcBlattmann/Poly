//! The NT object manager and dispatcher, as host-tested logic.
//!
//! This crate is what the NT personality's object services do, without the
//! machine: no threads, no clock, no user memory. The kernel keeps one
//! [`ObjectManager`] behind a lock, copies system call arguments in, calls
//! the matching `nt_*` service, and acts on what comes back.
//!
//! - **Objects** ([`ObjectType`]): directories, symbolic links, events,
//!   mutants, semaphores, timers, and opaque placeholders (sections, files,
//!   processes, threads, keys, devices) whose behaviour the kernel provides.
//!   Each has a handle count and a pointer count; see [`manager`] for when
//!   names go away and objects are deleted.
//! - **Namespace** ([`namespace`]): `\`, `\??`, `\GLOBAL??`, `\Device`,
//!   `\BaseNamedObjects`, `\Sessions`, `\KernelObjects`, symbolic links with a
//!   reparse limit, case-insensitive lookup ([`name`]), and a [`ParseHook`]
//!   for objects that own the names below them.
//! - **Handles** ([`handle`]): one table per process plus a kernel table,
//!   handle values that are multiples of 4, pseudo handles, inheritance,
//!   duplication and protection from close. Access masks ([`access`]) are
//!   checked when a handle is made and again when it is used.
//! - **Dispatcher** ([`wait`]): signal state rules per type and
//!   wait-any/wait-all over up to 64 objects. The crate decides which waits
//!   are satisfied ([`Wake`]); the kernel puts threads to sleep and wakes
//!   them, and ends waits for timeouts and alerts.
//!
//! # Sources and open questions
//!
//! Everything here comes from public documentation: Microsoft Learn (the
//! object manager, `OBJECT_ATTRIBUTES`, `InitializeObjectAttributes`, the
//! `Nt*`/`Zw*` routines, the Win32 synchronisation functions) and the
//! public SDK and WDK headers for constants (`ntstatus.h`, `winnt.h`,
//! `ntdef.h`, `wdm.h`). Where the documentation leaves behaviour open, the
//! code picks one answer and its comment says "to be confirmed by the spec
//! agent against real Windows"; ADR 0030 lists them.
//!
//! Allocation: the namespace, handle tables and wait queues grow without a
//! bound known in advance, so this crate uses `alloc` (the kernel has a
//! heap, ADR 0003's "allocation-free until the kernel heap exists" no longer
//! applies).

#![no_std]

extern crate alloc;

pub mod access;
pub mod handle;
pub mod manager;
pub mod name;
pub mod namespace;
mod object;
mod services;
mod slab;
pub mod status;
pub mod wait;

#[cfg(test)]
mod tests;

pub use access::{AccessMask, Mode, Privileges};
pub use handle::{Handle, HandleFlags};
pub use manager::{Deleted, ObjectManager};
pub use namespace::{Intent, NoParse, ParseHook, ParseRequest, Parsed};
pub use object::{EventType, ObjectId, ObjectType};
pub use services::ObjectBasicInformation;
pub use status::{NtStatus, Result};
pub use wait::{
    EventInfo, MAXIMUM_WAIT_OBJECTS, MutantInfo, SemaphoreInfo, ThreadRef, Timeout, WaitCancel, WaitId, WaitOutcome,
    WaitType, Wake,
};

/// A process, as the kernel names it. Each has its own handle table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProcessId(pub u32);

/// Who is calling a service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caller {
    pub process: ProcessId,
    pub thread: ThreadRef,
    /// The previous mode: `User` for system calls from user mode.
    pub mode: Mode,
    pub privileges: Privileges,
}

impl Caller {
    /// A user-mode caller without privileges.
    pub const fn user(process: ProcessId, thread: ThreadRef) -> Caller {
        Caller {
            process,
            thread,
            mode: Mode::User,
            privileges: Privileges { create_permanent: false, security: false },
        }
    }

    /// A kernel-mode caller acting in `process`.
    pub const fn kernel(process: ProcessId, thread: ThreadRef) -> Caller {
        Caller {
            process,
            thread,
            mode: Mode::Kernel,
            privileges: Privileges { create_permanent: true, security: true },
        }
    }
}

// `OBJ_*` attribute flags from `ntdef.h` (meanings: Learn, "OBJECT_ATTRIBUTES").

/// Handle attribute: protect the handle from `NtClose` (`wdm.h`). Only
/// meaningful in `NtDuplicateObject`'s handle attributes and
/// `NtQueryObject`; to be confirmed by the spec agent.
pub const OBJ_PROTECT_CLOSE: u32 = 0x0000_0001;
/// The handle is inherited by child processes.
pub const OBJ_INHERIT: u32 = 0x0000_0002;
/// A named object stays when its last handle closes.
pub const OBJ_PERMANENT: u32 = 0x0000_0010;
/// Only one process may hold handles to the object.
pub const OBJ_EXCLUSIVE: u32 = 0x0000_0020;
/// Compare names without case.
pub const OBJ_CASE_INSENSITIVE: u32 = 0x0000_0040;
/// Creating an object whose name exists opens it instead.
pub const OBJ_OPENIF: u32 = 0x0000_0080;
/// Open a symbolic link itself, not its target.
pub const OBJ_OPENLINK: u32 = 0x0000_0100;
/// The handle goes in the kernel handle table (kernel mode only).
pub const OBJ_KERNEL_HANDLE: u32 = 0x0000_0200;
/// Check access even for kernel-mode callers.
pub const OBJ_FORCE_ACCESS_CHECK: u32 = 0x0000_0400;
/// Do not use an impersonation token's device map (no effect yet).
pub const OBJ_IGNORE_IMPERSONATED_DEVICEMAP: u32 = 0x0000_0800;
/// Fail instead of following reparse points.
pub const OBJ_DONT_REPARSE: u32 = 0x0000_1000;
/// Every attribute bit `OBJECT_ATTRIBUTES` may carry.
pub const OBJ_VALID_ATTRIBUTES: u32 = 0x0000_1FF2;

/// `DUPLICATE_CLOSE_SOURCE` (`winnt.h`).
pub const DUPLICATE_CLOSE_SOURCE: u32 = 0x1;
/// `DUPLICATE_SAME_ACCESS` (`winnt.h`).
pub const DUPLICATE_SAME_ACCESS: u32 = 0x2;
/// `DUPLICATE_SAME_ATTRIBUTES` (Learn: "ZwDuplicateObject").
pub const DUPLICATE_SAME_ATTRIBUTES: u32 = 0x4;

/// `OBJECT_ATTRIBUTES` once the personality has captured it: the root
/// directory handle, the name (the `UNICODE_STRING`'s code units) and the
/// `OBJ_*` flags. Security descriptors and quality of service come later.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObjectAttributes<'a> {
    pub root: Option<Handle>,
    pub name: Option<&'a [u16]>,
    pub attributes: u32,
}

impl<'a> ObjectAttributes<'a> {
    pub const fn named(name: &'a [u16], attributes: u32) -> ObjectAttributes<'a> {
        ObjectAttributes { root: None, name: Some(name), attributes }
    }

    pub const fn unnamed(attributes: u32) -> ObjectAttributes<'a> {
        ObjectAttributes { root: None, name: None, attributes }
    }

    pub const fn relative(root: Handle, name: &'a [u16], attributes: u32) -> ObjectAttributes<'a> {
        ObjectAttributes { root: Some(root), name: Some(name), attributes }
    }
}

/// The object [`ObjectManager::create_object`] makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewObject<'a> {
    Directory,
    SymbolicLink {
        target: &'a [u16],
    },
    Event {
        kind: EventType,
        signaled: bool,
    },
    /// With `initial_owner` the calling thread owns it once.
    Mutant {
        initial_owner: bool,
    },
    /// `maximum` > 0 and 0 <= `initial` <= `maximum`, else
    /// `STATUS_INVALID_PARAMETER`.
    Semaphore {
        initial: i32,
        maximum: i32,
    },
    Timer {
        kind: EventType,
    },
    /// A placeholder of an opaque type. `signaled: Some(_)` makes it
    /// waitable with that initial state; `parse: true` sends lookups below
    /// it to the [`ParseHook`]; `cookie` comes back in [`Deleted`].
    Opaque {
        kind: ObjectType,
        cookie: u64,
        signaled: Option<bool>,
        parse: bool,
    },
}

/// A created (or, with `OBJ_OPENIF`, opened) object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Created {
    pub handle: Handle,
    pub object: ObjectId,
    /// `STATUS_SUCCESS`, or `STATUS_OBJECT_NAME_EXISTS` when `OBJ_OPENIF`
    /// opened an existing object.
    pub status: NtStatus,
}
