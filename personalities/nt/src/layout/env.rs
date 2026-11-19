//! The environment a new NT process gets (ADR 0073), and the environment
//! block format (Learn: "Environment Variables", "Changing Environment
//! Variables", `CreateProcessW`'s `lpEnvironment`,
//! `ExpandEnvironmentStringsW`).
//!
//! The rule itself, and the block, are `poly_kernelbase::defenv`'s, shared
//! with `userenv.dll`'s `CreateEnvironmentBlock` (which reads the registry
//! through the registry functions): this module reads the kernel's own
//! registry tree for it ([`build`]). See that module for the order of the
//! steps and the fallbacks.

use super::{
    COMMON_FILES, COMMON_FILES_X86, PROGRAM_DATA, PROGRAM_FILES, PROGRAM_FILES_X86, PUBLIC, SYSTEM_DRIVE, SYSTEM_ROOT,
    User,
};
use alloc::string::String;
use alloc::vec::Vec;
use poly_kernelbase::defenv::{self, Layout, Root, Source, UserInfo};
use poly_ntobj::name::wide;
use poly_registry::Registry;
use poly_registry::consts::{REG_EXPAND_SZ, REG_SZ};
use poly_registry::tree::KeyId;

pub use poly_kernelbase::defenv::{
    Block, CURRENT_VERSION, Machine, NT_CURRENT_VERSION, PROFILE_LIST, Processor, compare_names,
};

/// The layout's folders as the default environment's fallbacks (equal to
/// [`defenv::POLY_LAYOUT`], which `userenv.dll` uses; the host tests check it).
pub const LAYOUT: Layout = Layout {
    system_drive: SYSTEM_DRIVE,
    system_root: SYSTEM_ROOT,
    program_files: PROGRAM_FILES,
    program_files_x86: PROGRAM_FILES_X86,
    common_files: COMMON_FILES,
    common_files_x86: COMMON_FILES_X86,
    program_data: PROGRAM_DATA,
    public: PUBLIC,
    computer_name: poly_registry::defaults::POLY_COMPUTER_NAME,
};

/// The string value `name` of the key at `path` below `start`: its type
/// and text.
pub fn string_value(reg: &Registry, start: KeyId, path: &str, name: &str) -> Option<(u32, String)> {
    let key = reg.find(start, path)?;
    let v = reg.value(key, &wide(name)).ok()?;
    matches!(v.kind, REG_SZ | REG_EXPAND_SZ).then(|| (v.kind, defenv::text_of(&v.data)))
}

/// `SYSTEM\ControlSet00n`, the control set `SYSTEM\Select`'s `Current`
/// names (what the `CurrentControlSet` link points at); `ControlSet001`
/// when there is none.
pub fn current_control_set(reg: &Registry) -> String {
    defenv::current_control_set(&Tree(reg))
}

/// The kernel's registry as the default environment's [`Source`].
struct Tree<'a>(&'a Registry);

impl Tree<'_> {
    fn root(&self, root: Root) -> KeyId {
        match root {
            Root::Machine => self.0.machine(),
            Root::Users => self.0.user(),
        }
    }
}

impl Source for Tree<'_> {
    fn value(&self, root: Root, path: &str, name: &str) -> Option<(u32, Vec<u8>)> {
        let key = self.0.find(self.root(root), path)?;
        let v = self.0.value(key, &wide(name)).ok()?;
        Some((v.kind, v.data.clone()))
    }

    fn values(&self, root: Root, path: &str) -> Vec<(String, u32, Vec<u8>)> {
        let Some(key) = self.0.find(self.root(root), path) else { return Vec::new() };
        let Ok(k) = self.0.key(key) else { return Vec::new() };
        k.values().iter().map(|v| (String::from_utf16_lossy(&v.name), v.kind, v.data.clone())).collect()
    }
}

/// The default environment of a new NT process of `user` (see
/// `poly_kernelbase::defenv` for the order). Kernel-made processes run in
/// session 0.
pub fn build(reg: &Registry, user: &User, machine: Machine) -> Block {
    let info = UserInfo { name: user.name.into(), sid: user.sid.into(), profile: user.profile(), session: 0 };
    defenv::build(&Tree(reg), &LAYOUT, Some(&info), machine, Block::new())
}
