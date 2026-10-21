//! The NT registry, as host-tested logic (ADR 0066).
//!
//! This crate is what the NT registry services do, without the machine: no
//! user memory, no handles, no files. The NT personality keeps one
//! [`Registry`] next to the object manager, reaches it through the
//! `\REGISTRY` parse object, copies arguments in and the structures this
//! crate encodes out, and the kernel saves and loads the hive files.
//!
//! - **Keys and values** ([`tree`]): case-insensitive, case-preserving
//!   names; values of every `REG_*` type (any type number is stored);
//!   volatile keys; symbolic-link keys; deletion while open; last-write
//!   times. Open instances ([`OpenId`]) stand for key objects.
//! - **Information classes** ([`info`]): `KEY_*_INFORMATION` and
//!   `KEY_VALUE_*_INFORMATION` as `wdm.h` lays them out, and the
//!   `STATUS_BUFFER_OVERFLOW` / `STATUS_BUFFER_TOO_SMALL` rules.
//! - **Notifications** ([`notify`]): `NtNotifyChangeKey` watches with their
//!   filters and subtree flag.
//! - **Hive files** ([`hive`]): Poly's own versioned snapshot format. Not
//!   Microsoft's regf format, which is not publicly documented.
//! - **Default hives** ([`defaults`]): `HKLM\SYSTEM`, `HKLM\SOFTWARE`,
//!   `HKU\.DEFAULT` and Poly's user, plus the volatile keys of every boot.
//!
//! # Sources
//!
//! Microsoft Learn's pages for `ZwCreateKey`, `ZwOpenKeyEx`, `ZwQueryKey`,
//! `ZwEnumerateKey`, `ZwQueryValueKey`, `ZwEnumerateValueKey`,
//! `ZwSetValueKey`, `ZwDeleteKey`, `ZwDeleteValueKey`, `ZwFlushKey`,
//! `ZwNotifyChangeKey`, the `KEY_*` structures and enumerations, the Win32
//! `Reg*` functions, "Registry Value Types", "Registry Element Size
//! Limits" and "Predefined Keys"; constants from `winnt.h`, `wdm.h`,
//! `ntddk.h` and `ntstatus.h`. Where they leave behaviour open the code
//! picks one answer and says "to be confirmed by the spec agent against
//! real Windows"; ADR 0066 lists them. Security descriptors are not
//! modelled: Poly has one user, and the only access checks are those of the
//! handle's granted access (ADR 0030's allowed mask grants everything).

#![no_std]

extern crate alloc;

pub mod consts;
pub mod defaults;
pub mod hive;
pub mod info;
pub mod notify;
mod slab;
pub mod tree;

#[cfg(test)]
mod tests;

pub use notify::Fired;
pub use tree::{CreateOptions, Disposition, Hive, HiveId, Key, KeyId, KeyInfo, OpenId, Registry, Resolved, Value};
