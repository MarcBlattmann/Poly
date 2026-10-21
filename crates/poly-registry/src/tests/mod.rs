//! Host tests of the registry logic.

extern crate std;

mod defaults;
mod hive;
mod info;
mod keys;
mod notify;

use crate::consts::*;
use crate::tree::{CreateOptions, KeyId, Registry};
use alloc::string::String;
use alloc::vec::Vec;
use poly_ntobj::name::wide;

pub const T0: u64 = FILETIME_UNIX_EPOCH + 1_000_000_000 * 10_000_000;

/// A registry with one empty hive at `\REGISTRY\MACHINE\TEST`.
pub fn with_hive() -> (Registry, KeyId) {
    let mut r = Registry::new();
    let m = r.machine();
    let h = r.mount_hive(m, &wide("TEST"), "TEST", None, T0).unwrap();
    let root = r.hive(h).root;
    (r, root)
}

pub fn create(r: &mut Registry, parent: KeyId, name: &str) -> KeyId {
    r.create_key(parent, &wide(name), CreateOptions::default(), T0).unwrap().0
}

pub fn create_volatile(r: &mut Registry, parent: KeyId, name: &str) -> KeyId {
    r.create_key(parent, &wide(name), CreateOptions { volatile: true, ..Default::default() }, T0).unwrap().0
}

pub fn names(r: &Registry, key: KeyId) -> Vec<String> {
    (0..)
        .map_while(|i| r.subkey_at(key, i).ok())
        .map(|k| String::from_utf16(r.key(k).unwrap().name()).unwrap())
        .collect()
}

pub fn sz(s: &str) -> Vec<u8> {
    crate::defaults::sz(s)
}

pub fn link_data(target: &str) -> Vec<u8> {
    target.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

pub fn path_string(r: &Registry, k: KeyId) -> String {
    String::from_utf16(&r.full_path(k).unwrap()).unwrap()
}
