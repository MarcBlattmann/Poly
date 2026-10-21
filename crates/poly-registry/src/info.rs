//! The information structures of `NtQueryKey`, `NtEnumerateKey`,
//! `NtQueryValueKey` and `NtEnumerateValueKey`, laid out as `wdm.h` and
//! `ntddk.h` declare them (Learn: "KEY_BASIC_INFORMATION",
//! "KEY_NODE_INFORMATION", "KEY_FULL_INFORMATION", "KEY_NAME_INFORMATION",
//! "KEY_CACHED_INFORMATION", "KEY_VIRTUALIZATION_INFORMATION",
//! "KEY_VALUE_BASIC_INFORMATION", "KEY_VALUE_FULL_INFORMATION",
//! "KEY_VALUE_PARTIAL_INFORMATION"), and the buffer-size rules.
//!
//! # Buffer sizes (Learn: "ZwQueryKey", "ZwQueryValueKey", ...)
//!
//! `ResultLength` always receives the size of the whole structure with its
//! strings and data. A buffer at least that big gets it all
//! (`STATUS_SUCCESS`). A buffer that holds the fixed part but not the rest
//! gets the fixed part only ("only partial data has been written",
//! `STATUS_BUFFER_OVERFLOW`); a smaller one gets nothing
//! (`STATUS_BUFFER_TOO_SMALL`). That the partial data is exactly the fixed
//! part (and not also part of the name) is to be confirmed by the spec
//! agent against real Windows.
//!
//! Strings are not NUL-terminated (Learn says so of every name and class
//! in these structures). `TitleIndex` is always 0. Where a class string is
//! empty, `ClassOffset` still points where it would start (to be
//! confirmed). `KEY_VALUE_FULL_INFORMATION.DataOffset` is the offset just
//! after the name ("the data immediately following the Name string"); in
//! the `Align64` variant it is rounded up to 8.

use crate::consts::*;
use crate::tree::{KeyId, Registry, Result, Value};
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::status::*;

/// A structure ready to copy out: its bytes and the size of its fixed
/// part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Encoded {
    pub bytes: Vec<u8>,
    pub fixed: usize,
}

/// What to copy for a buffer of some length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fit<'a> {
    pub status: NtStatus,
    /// The bytes to write at the start of the buffer (maybe none).
    pub write: &'a [u8],
    /// What goes to `ResultLength`.
    pub result_length: u32,
}

/// Apply the buffer-size rules to a buffer of `length` bytes.
pub fn fit(e: &Encoded, length: usize) -> Fit<'_> {
    let result_length = e.bytes.len() as u32;
    if length >= e.bytes.len() {
        Fit { status: STATUS_SUCCESS, write: &e.bytes, result_length }
    } else if length >= e.fixed {
        Fit { status: STATUS_BUFFER_OVERFLOW, write: &e.bytes[..e.fixed], result_length }
    } else {
        Fit { status: STATUS_BUFFER_TOO_SMALL, write: &[], result_length }
    }
}

fn u32s(out: &mut Vec<u8>, values: &[u32]) {
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn text(out: &mut Vec<u8>, s: &[u16]) {
    for c in s {
        out.extend_from_slice(&c.to_le_bytes());
    }
}

fn bytes_of(s: &[u16]) -> u32 {
    (s.len() * 2) as u32
}

/// Sizes of the fixed parts.
pub const KEY_BASIC_INFORMATION_SIZE: usize = 16;
pub const KEY_NODE_INFORMATION_SIZE: usize = 24;
pub const KEY_FULL_INFORMATION_SIZE: usize = 44;
pub const KEY_NAME_INFORMATION_SIZE: usize = 4;
/// `sizeof(KEY_CACHED_INFORMATION)`: 36 bytes of members, padded to the
/// 8-byte alignment of its `LARGE_INTEGER` (to be confirmed by the spec
/// agent).
pub const KEY_CACHED_INFORMATION_SIZE: usize = 40;
pub const KEY_VIRTUALIZATION_INFORMATION_SIZE: usize = 4;
pub const KEY_VALUE_BASIC_INFORMATION_SIZE: usize = 12;
pub const KEY_VALUE_FULL_INFORMATION_SIZE: usize = 20;
pub const KEY_VALUE_PARTIAL_INFORMATION_SIZE: usize = 12;
pub const KEY_VALUE_PARTIAL_INFORMATION_ALIGN64_SIZE: usize = 8;
pub const KEY_VALUE_LAYER_INFORMATION_SIZE: usize = 4;

/// The access `NtQueryKey` needs for `class` (Learn: "ZwQueryKey":
/// `KEY_QUERY_VALUE`, except for `KeyNameInformation` and
/// `KeyHandleTagsInformation`).
pub const fn query_key_access(class: u32) -> u32 {
    match class {
        KEY_NAME_INFORMATION | KEY_HANDLE_TAGS_INFORMATION => 0,
        _ => KEY_QUERY_VALUE,
    }
}

/// `NtQueryKey` (`enumerate: false`) or `NtEnumerateKey` (`true`, for a
/// subkey) information of class `class` about `key`. `NtEnumerateKey`
/// takes only the basic, node and full classes (Learn: "ZwEnumerateKey");
/// `NtQueryKey` also the name, cached and virtualization ones. Any other
/// class is `STATUS_INVALID_PARAMETER` (Learn; the reserved classes are
/// refused too, to be confirmed by the spec agent).
pub fn key_information(reg: &Registry, key: KeyId, class: u32, enumerate: bool) -> Result<Encoded> {
    let allowed = match class {
        KEY_BASIC_INFORMATION | KEY_NODE_INFORMATION | KEY_FULL_INFORMATION => true,
        KEY_NAME_INFORMATION | KEY_CACHED_INFORMATION | KEY_VIRTUALIZATION_INFORMATION => !enumerate,
        _ => false,
    };
    if !allowed {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let info = reg.info(key)?;
    let lw = info.last_write.to_le_bytes();
    let mut out = Vec::new();
    let fixed = match class {
        KEY_BASIC_INFORMATION => {
            out.extend_from_slice(&lw);
            u32s(&mut out, &[0, bytes_of(&info.name)]);
            text(&mut out, &info.name);
            KEY_BASIC_INFORMATION_SIZE
        }
        KEY_NODE_INFORMATION => {
            out.extend_from_slice(&lw);
            let class_offset = KEY_NODE_INFORMATION_SIZE as u32 + bytes_of(&info.name);
            u32s(&mut out, &[0, class_offset, bytes_of(&info.class), bytes_of(&info.name)]);
            text(&mut out, &info.name);
            text(&mut out, &info.class);
            KEY_NODE_INFORMATION_SIZE
        }
        KEY_FULL_INFORMATION => {
            out.extend_from_slice(&lw);
            u32s(
                &mut out,
                &[
                    0,
                    KEY_FULL_INFORMATION_SIZE as u32,
                    bytes_of(&info.class),
                    info.subkeys,
                    info.max_name_len,
                    info.max_class_len,
                    info.values,
                    info.max_value_name_len,
                    info.max_value_data_len,
                ],
            );
            text(&mut out, &info.class);
            KEY_FULL_INFORMATION_SIZE
        }
        KEY_NAME_INFORMATION => {
            let path = reg.full_path(key)?;
            u32s(&mut out, &[bytes_of(&path)]);
            text(&mut out, &path);
            KEY_NAME_INFORMATION_SIZE
        }
        KEY_CACHED_INFORMATION => {
            out.extend_from_slice(&lw);
            u32s(
                &mut out,
                &[
                    0,
                    info.subkeys,
                    info.max_name_len,
                    info.values,
                    info.max_value_name_len,
                    info.max_value_data_len,
                    bytes_of(&info.name),
                ],
            );
            out.resize(KEY_CACHED_INFORMATION_SIZE, 0);
            KEY_CACHED_INFORMATION_SIZE
        }
        KEY_VIRTUALIZATION_INFORMATION => {
            // No registry virtualization: every bit clear.
            u32s(&mut out, &[0]);
            KEY_VIRTUALIZATION_INFORMATION_SIZE
        }
        _ => unreachable!("checked above"),
    };
    Ok(Encoded { bytes: out, fixed })
}

/// `NtQueryValueKey` / `NtEnumerateValueKey` information of class `class`
/// about `value`. A class outside `KEY_VALUE_INFORMATION_CLASS` is
/// `STATUS_INVALID_PARAMETER` (Learn). `KeyValueLayerInformation` is a
/// `ULONG` of flags, all clear (no layered keys).
pub fn value_information(value: &Value, class: u32) -> Result<Encoded> {
    let name = &value.name;
    let data = &value.data;
    let mut out = Vec::new();
    let fixed = match class {
        KEY_VALUE_BASIC_INFORMATION => {
            u32s(&mut out, &[0, value.kind, bytes_of(name)]);
            text(&mut out, name);
            KEY_VALUE_BASIC_INFORMATION_SIZE
        }
        KEY_VALUE_FULL_INFORMATION | KEY_VALUE_FULL_INFORMATION_ALIGN64 => {
            let mut offset = KEY_VALUE_FULL_INFORMATION_SIZE + name.len() * 2;
            if class == KEY_VALUE_FULL_INFORMATION_ALIGN64 {
                offset = offset.div_ceil(8) * 8;
            }
            u32s(&mut out, &[0, value.kind, offset as u32, data.len() as u32, bytes_of(name)]);
            text(&mut out, name);
            out.resize(offset, 0);
            out.extend_from_slice(data);
            KEY_VALUE_FULL_INFORMATION_SIZE
        }
        KEY_VALUE_PARTIAL_INFORMATION => {
            u32s(&mut out, &[0, value.kind, data.len() as u32]);
            out.extend_from_slice(data);
            KEY_VALUE_PARTIAL_INFORMATION_SIZE
        }
        KEY_VALUE_PARTIAL_INFORMATION_ALIGN64 => {
            u32s(&mut out, &[value.kind, data.len() as u32]);
            out.extend_from_slice(data);
            KEY_VALUE_PARTIAL_INFORMATION_ALIGN64_SIZE
        }
        KEY_VALUE_LAYER_INFORMATION => {
            u32s(&mut out, &[0]);
            KEY_VALUE_LAYER_INFORMATION_SIZE
        }
        _ => return Err(STATUS_INVALID_PARAMETER),
    };
    Ok(Encoded { bytes: out, fixed })
}

/// Whether `class` needs an 8-byte aligned buffer (Learn:
/// "KEY_VALUE_INFORMATION_CLASS": otherwise
/// `STATUS_DATATYPE_MISALIGNMENT`).
pub const fn value_class_needs_align64(class: u32) -> bool {
    matches!(class, KEY_VALUE_FULL_INFORMATION_ALIGN64 | KEY_VALUE_PARTIAL_INFORMATION_ALIGN64)
}
