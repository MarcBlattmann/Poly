//! The link's property store (the sheet `ShellLink.PropertyStore.yaml`) and its form in the file:
//! the `PropertyStoreDataBlock` (signature 0xA0000009, MS-SHLLINK 2.5.7) holds a serialized
//! property store (MS-PROPSTORE 2.2): a run of serialized property storages, each its size, the
//! version 0x53505331, a format id and its values (a value: its size, the property id, a reserved
//! byte and a typed value of MS-OLEPS 2.15), ended by a value size of 0; the run ends with a
//! storage size of 0.
//!
//! The store keeps any key with any value in the order the keys were first set; `VT_EMPTY` keeps
//! the key (the sheet's `empty`).

use crate::idlist::{guid_bytes, guid_from};
use crate::{Guid, Text};
use alloc::vec::Vec;

/// `PROPERTYKEY` (`wtypes.h`): a format id and a property id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropKey {
    pub fmtid: Guid,
    pub pid: u32,
}

impl PropKey {
    /// The key from the 20 bytes of a `PROPERTYKEY` in memory.
    pub fn from_bytes(b: &[u8; 20]) -> PropKey {
        PropKey { fmtid: guid_from(b).unwrap_or_default(), pid: u32::from_le_bytes([b[16], b[17], b[18], b[19]]) }
    }

    /// The 20 bytes of the key in memory.
    pub fn to_bytes(&self) -> [u8; 20] {
        let mut b = [0u8; 20];
        b[..16].copy_from_slice(&guid_bytes(&self.fmtid));
        b[16..].copy_from_slice(&self.pid.to_le_bytes());
        b
    }
}

const fn key(data1: u32, data2: u16, data3: u16, data4: [u8; 8], pid: u32) -> PropKey {
    PropKey { fmtid: Guid { data1, data2, data3, data4 }, pid }
}

/// `PKEY_AppUserModel_ID` (`propkey.h`): {9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3}, 5.
pub const PKEY_APP_USER_MODEL_ID: PropKey =
    key(0x9F4C_2855, 0x9F79, 0x4B39, [0xA8, 0xD0, 0xE1, 0xD4, 0x2D, 0xE1, 0xD5, 0xF3], 5);
/// `PKEY_AppUserModel_ToastActivatorCLSID` (`propkey.h`): the same format id, 26.
pub const PKEY_TOAST_ACTIVATOR_CLSID: PropKey =
    key(0x9F4C_2855, 0x9F79, 0x4B39, [0xA8, 0xD0, 0xE1, 0xD4, 0x2D, 0xE1, 0xD5, 0xF3], 26);
/// `PKEY_VolumeId` (`propkey.h`): {446D16B1-8DAD-4870-A748-402EA43D788C}, 104.
pub const PKEY_VOLUME_ID: PropKey =
    key(0x446D_16B1, 0x8DAD, 0x4870, [0xA7, 0x48, 0x40, 0x2E, 0xA4, 0x3D, 0x78, 0x8C], 104);
/// `PKEY_Comment` (`propkey.h`): {F29F85E0-4FF9-1068-AB91-08002B27B3D9}, 6.
pub const PKEY_COMMENT: PropKey = key(0xF29F_85E0, 0x4FF9, 0x1068, [0xAB, 0x91, 0x08, 0x00, 0x2B, 0x27, 0xB3, 0xD9], 6);
/// The entry Save adds, a string with the user's SID (the sheet's `persist`):
/// {46588AE2-4CBC-4338-BBFC-139326986DCE}, 4.
pub const PKEY_SAVED_BY_SID: PropKey =
    key(0x4658_8AE2, 0x4CBC, 0x4338, [0xBB, 0xFC, 0x13, 0x93, 0x26, 0x98, 0x6D, 0xCE], 4);

/// `VARENUM` values (`wtypes.h`) the store handles by name.
pub const VT_EMPTY: u16 = 0;
pub const VT_I2: u16 = 2;
pub const VT_I4: u16 = 3;
pub const VT_R4: u16 = 4;
pub const VT_R8: u16 = 5;
pub const VT_CY: u16 = 6;
pub const VT_DATE: u16 = 7;
pub const VT_ERROR: u16 = 10;
pub const VT_BOOL: u16 = 11;
pub const VT_I1: u16 = 16;
pub const VT_UI1: u16 = 17;
pub const VT_UI2: u16 = 18;
pub const VT_UI4: u16 = 19;
pub const VT_I8: u16 = 20;
pub const VT_UI8: u16 = 21;
pub const VT_INT: u16 = 22;
pub const VT_UINT: u16 = 23;
pub const VT_LPWSTR: u16 = 31;
pub const VT_FILETIME: u16 = 64;
pub const VT_CLSID: u16 = 72;

/// The bytes a scalar type takes in a `PROPVARIANT` and in a serialized value (`None`: not a
/// scalar).
pub fn scalar_size(vt: u16) -> Option<usize> {
    match vt {
        VT_I1 | VT_UI1 => Some(1),
        VT_I2 | VT_UI2 | VT_BOOL => Some(2),
        VT_I4 | VT_UI4 | VT_R4 | VT_INT | VT_UINT | VT_ERROR => Some(4),
        VT_I8 | VT_UI8 | VT_R8 | VT_CY | VT_DATE | VT_FILETIME => Some(8),
        _ => None,
    }
}

/// A value of the store.
#[derive(Clone, Debug, PartialEq)]
pub enum PropValue {
    /// `VT_EMPTY`.
    Empty,
    /// `VT_LPWSTR`.
    Text(Text),
    /// `VT_CLSID`.
    Clsid(Guid),
    /// A scalar type ([`scalar_size`]) and the 8 bytes at offset 8 of its `PROPVARIANT`.
    Scalar(u16, [u8; 8]),
}

/// The keys and values of a link, in the order the keys were first set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Store {
    entries: Vec<(PropKey, PropValue)>,
}

impl Store {
    pub const fn new() -> Store {
        Store { entries: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn key_at(&self, index: usize) -> Option<PropKey> {
        self.entries.get(index).map(|(k, _)| *k)
    }

    pub fn get(&self, key: &PropKey) -> Option<&PropValue> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Set a value: a key the store has keeps its place.
    pub fn set(&mut self, key: &PropKey, value: PropValue) {
        match self.entries.iter_mut().find(|(k, _)| k == key) {
            Some(e) => e.1 = value,
            None => self.entries.push((*key, value)),
        }
    }

    pub fn entries(&self) -> &[(PropKey, PropValue)] {
        &self.entries
    }

    /// The serialized property store of the file (without the block header): the values grouped
    /// by format id in the order the store has them, the empty ones left out; `sid` (the user's
    /// SID, which Save adds) goes after the storages of the volume id and the AppUserModel
    /// properties when the store does not have it. With nothing to write it is the 4-byte end alone.
    pub fn serialize(&self, sid: Option<&[u16]>) -> Vec<u8> {
        let mut order: Vec<Guid> = Vec::new();
        for (k, v) in &self.entries {
            if *v != PropValue::Empty && !order.contains(&k.fmtid) {
                order.push(k.fmtid);
            }
        }
        let sid_value = sid.filter(|_| self.get(&PKEY_SAVED_BY_SID).is_none()).map(|s| PropValue::Text(s.to_vec()));
        if sid_value.is_some() {
            let after = order
                .iter()
                .rposition(|g| *g == PKEY_VOLUME_ID.fmtid || *g == PKEY_APP_USER_MODEL_ID.fmtid)
                .map_or(0, |i| i + 1);
            order.insert(after, PKEY_SAVED_BY_SID.fmtid);
        }
        let mut out = Vec::new();
        for fmtid in order {
            let start = out.len();
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&0x5350_5331u32.to_le_bytes());
            out.extend_from_slice(&guid_bytes(&fmtid));
            let values = self.entries.iter().filter(|(k, _)| k.fmtid == fmtid).map(|(k, v)| (k.pid, v));
            let extra =
                sid_value.as_ref().filter(|_| fmtid == PKEY_SAVED_BY_SID.fmtid).map(|v| (PKEY_SAVED_BY_SID.pid, v));
            for (pid, v) in values.chain(extra) {
                let Some(typed) = typed_value(v) else { continue };
                out.extend_from_slice(&((9 + typed.len()) as u32).to_le_bytes());
                out.extend_from_slice(&pid.to_le_bytes());
                out.push(0);
                out.extend_from_slice(&typed);
            }
            out.extend_from_slice(&[0; 4]);
            let size = (out.len() - start) as u32;
            out[start..start + 4].copy_from_slice(&size.to_le_bytes());
        }
        out.extend_from_slice(&[0; 4]);
        out
    }

    /// The store a serialized property store holds. Each value read goes in front of the ones
    /// read before it (a loaded link lists its values in the reverse order of the file, as the
    /// sheet's `persist` shows). Storages with string names and values of other types are
    /// skipped; a store cut short gives what was whole.
    pub fn deserialize(data: &[u8]) -> Store {
        let mut store = Store::new();
        let mut at = 0;
        while let Some(size) = read_u32(data, at).map(|s| s as usize) {
            if size < 24 || at + size > data.len() {
                break;
            }
            let storage = &data[at..at + size];
            at += size;
            if read_u32(storage, 4) != Some(0x5350_5331) {
                continue;
            }
            let Some(fmtid) = guid_from(&storage[8..24]) else { continue };
            // FMTID_Storage-style string names (D5CDD505-2E9C-101B-9397-08002B2CF9AE) are not kept.
            if fmtid
                == (Guid {
                    data1: 0xD5CD_D505,
                    data2: 0x2E9C,
                    data3: 0x101B,
                    data4: [0x93, 0x97, 0x08, 0x00, 0x2B, 0x2C, 0xF9, 0xAE],
                })
            {
                continue;
            }
            let mut v = 24;
            while let Some(vsize) = read_u32(storage, v).map(|s| s as usize) {
                if vsize < 9 || v + vsize > storage.len() {
                    break;
                }
                let pid = read_u32(storage, v + 4).unwrap_or(0);
                if let Some(value) = read_typed(&storage[v + 9..v + vsize]) {
                    let key = PropKey { fmtid, pid };
                    store.entries.retain(|(k, _)| *k != key);
                    store.entries.insert(0, (key, value));
                }
                v += vsize;
            }
        }
        store
    }
}

fn read_u32(d: &[u8], at: usize) -> Option<u32> {
    let b = d.get(at..at + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// A typed property value (MS-OLEPS 2.15): the type (2), 2 bytes of padding, the value padded to
/// a multiple of 4 bytes. A string is its length in characters with the NUL (4), then UTF-16.
fn typed_value(v: &PropValue) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    match v {
        PropValue::Empty => return None,
        PropValue::Text(t) => {
            out.extend_from_slice(&VT_LPWSTR.to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(&((t.len() + 1) as u32).to_le_bytes());
            for c in t.iter().chain(&[0]) {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        PropValue::Clsid(g) => {
            out.extend_from_slice(&VT_CLSID.to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(&guid_bytes(g));
        }
        PropValue::Scalar(vt, bytes) => {
            let n = scalar_size(*vt)?;
            out.extend_from_slice(&vt.to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(&bytes[..n]);
        }
    }
    while out.len() % 4 != 0 {
        out.push(0);
    }
    Some(out)
}

fn read_typed(d: &[u8]) -> Option<PropValue> {
    let vt = u16::from_le_bytes([*d.first()?, *d.get(1)?]);
    let body = d.get(4..)?;
    match vt {
        VT_EMPTY => Some(PropValue::Empty),
        VT_LPWSTR => {
            let n = read_u32(body, 0)? as usize;
            let units = body.get(4..4 + 2 * n)?;
            let mut t: Text = units.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            if let Some(end) = t.iter().position(|&c| c == 0) {
                t.truncate(end);
            }
            Some(PropValue::Text(t))
        }
        VT_CLSID => guid_from(body).map(PropValue::Clsid),
        _ => {
            let n = scalar_size(vt)?;
            let mut bytes = [0u8; 8];
            bytes[..n].copy_from_slice(body.get(..n)?);
            Some(PropValue::Scalar(vt, bytes))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::wide;

    #[test]
    fn the_chrome_store_is_the_size_the_oracle_wrote() {
        // AppUserModel.ID "Chrome" and the SID of Poly's user: the oracle's block of 210 bytes is
        // the 8-byte block header and this store.
        let mut s = Store::new();
        s.set(&PKEY_APP_USER_MODEL_ID, PropValue::Text(wide("Chrome")));
        let sid = wide("S-1-5-21-520193847-884726105-1936024571-1002");
        assert_eq!(8 + s.serialize(Some(&sid)).len(), 210);
        // With a volume id (an existing target): 57 bytes more (1975 - 1708 = 267).
        s.set(&PKEY_VOLUME_ID, PropValue::Clsid(crate::volume_guid(7)));
        assert_eq!(8 + s.serialize(Some(&sid)).len(), 267);
    }

    #[test]
    fn a_store_reads_back_in_reverse_order() {
        let custom = key(0x5E1B_0001, 1, 2, [1, 2, 3, 4, 5, 6, 7, 8], 7);
        let target = key(0xB9B4_B3FC, 0x2B51, 0x4A42, [0xB5, 0xD8, 0x32, 0x41, 0x46, 0xAF, 0xCF, 0x25], 2);
        let toast =
            Guid { data1: 0xA6B5_E3B5, data2: 0, data3: 0x4A0B, data4: [0x80, 0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66] };
        let mut s = Store::new();
        s.set(&PKEY_VOLUME_ID, PropValue::Clsid(crate::volume_guid(7)));
        s.set(&PKEY_APP_USER_MODEL_ID, PropValue::Text(wide("PolySpecB4.App")));
        s.set(&PKEY_TOAST_ACTIVATOR_CLSID, PropValue::Clsid(toast));
        s.set(&custom, PropValue::Text(wide("custom value")));
        s.set(&target, PropValue::Text(wide("C:\\x\\y.exe")));
        let back = Store::deserialize(&s.serialize(Some(&wide("S-1-5-21-1"))));
        let keys: Vec<PropKey> = (0..back.len()).filter_map(|i| back.key_at(i)).collect();
        // The order the sheet's ps_aumid lists after Load.
        assert_eq!(
            keys,
            [target, custom, PKEY_SAVED_BY_SID, PKEY_TOAST_ACTIVATOR_CLSID, PKEY_APP_USER_MODEL_ID, PKEY_VOLUME_ID]
        );
        assert_eq!(back.get(&PKEY_TOAST_ACTIVATOR_CLSID), Some(&PropValue::Clsid(toast)));
        assert_eq!(back.get(&custom), Some(&PropValue::Text(wide("custom value"))));
        // Saved again, the SID is not added twice.
        assert_eq!(Store::deserialize(&back.serialize(Some(&wide("S-1-5-21-1")))).len(), 6);
    }

    #[test]
    fn scalars_and_empty_values() {
        let mut s = Store::new();
        s.set(&PKEY_APP_USER_MODEL_ID, PropValue::Scalar(VT_UI4, [5, 0, 0, 0, 0, 0, 0, 0]));
        s.set(&PKEY_COMMENT, PropValue::Empty);
        assert_eq!(s.len(), 2);
        let back = Store::deserialize(&s.serialize(None));
        assert_eq!(back.len(), 1);
        assert_eq!(back.get(&PKEY_APP_USER_MODEL_ID), Some(&PropValue::Scalar(VT_UI4, [5, 0, 0, 0, 0, 0, 0, 0])));
        assert_eq!(Store::deserialize(&[1, 2]).len(), 0);
    }

    #[test]
    fn keys_in_memory() {
        let b = PKEY_APP_USER_MODEL_ID.to_bytes();
        assert_eq!(&b[..4], &[0x55, 0x28, 0x4C, 0x9F]);
        assert_eq!(PropKey::from_bytes(&b), PKEY_APP_USER_MODEL_ID);
    }
}
