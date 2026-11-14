//! A window's property store (ADR 0155, step 9): what `SHGetPropertyStoreForWindow` hands
//! out. Chrome sets its application user model id on every top-level window through it and
//! treats a failure as fatal.
//!
//! Sources: Learn, "SHGetPropertyStoreForWindow function (shellapi.h)" ("retrieves an object
//! that represents a specific window's collection of properties, which allows those properties
//! to be queried or set"), "IPropertyStore" and its five methods (`GetCount`, `GetAt`,
//! `GetValue`: a key the store does not have gives `VT_EMPTY` and `S_OK`; `SetValue`;
//! `Commit`), "PROPERTYKEY" (a `GUID` and a `DWORD`, 20 bytes) and "PROPVARIANT" (`propidlbase.h`:
//! 24 bytes on x64, the type in the first two). Poly keeps the properties of a window in the
//! process that set them; nothing in Poly's shell reads them yet. Doc-derived, not observed.

use alloc::vec::Vec;

/// `VARENUM` types (`wtypes.h`) the store keeps.
pub const VT_EMPTY: u16 = 0;
pub const VT_BSTR: u16 = 8;
pub const VT_LPSTR: u16 = 30;
pub const VT_LPWSTR: u16 = 31;
/// `VT_VECTOR`, `VT_ARRAY`, `VT_BYREF` (`wtypes.h`): values that own or point to memory.
pub const VT_INDIRECT_MASK: u16 = 0xF000;

/// `S_OK`, `E_INVALIDARG`, `E_NOINTERFACE`, `E_POINTER` (`winerror.h`).
pub const S_OK: u32 = 0;
pub const E_INVALIDARG: u32 = 0x8007_0057;
pub const E_NOINTERFACE: u32 = 0x8000_4002;
pub const E_POINTER: u32 = 0x8000_4003;
pub const E_OUTOFMEMORY: u32 = 0x8007_000E;
/// `E_FAIL` (`winerror.h`): `GetCount` of a store nothing was ever set in.
pub const E_FAIL: u32 = 0x8000_4005;
/// `HRESULT_FROM_WIN32(ERROR_INVALID_WINDOW_HANDLE)`: `SetValue` on the store of a window that
/// is gone.
pub const E_INVALID_WINDOW: u32 = 0x8007_0578;

/// `IID_IPropertyStore` {886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99} (`propsys.h`), in memory order.
pub const IID_IPROPERTYSTORE: [u8; 16] =
    [0xEB, 0x8E, 0x6D, 0x88, 0xF2, 0x8C, 0x46, 0x44, 0x8D, 0x02, 0xCD, 0xBA, 0x1D, 0xBD, 0xCF, 0x99];

/// A `PROPERTYKEY`: the format id and the property id, as its 20 bytes.
pub type Key = [u8; 20];

/// How a `PROPVARIANT` of type `vt` is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `VT_EMPTY`: setting it removes the property.
    Empty,
    /// The value is in the 24 bytes themselves (numbers, `VT_BOOL`, `VT_FILETIME`, ...).
    Scalar,
    /// `VT_LPWSTR`: a NUL-terminated UTF-16 string the value points to; the store keeps a copy.
    Text,
    /// A type that owns other memory (`VT_BSTR`, `VT_LPSTR`, `VT_CLSID`, `VT_BLOB`, vectors,
    /// arrays, references, interfaces): not kept, `E_INVALIDARG`.
    Unsupported,
}

/// The kind of a variant type. The scalar types are the ones `wtypes.h` lists as valid in a
/// `PROPVARIANT` whose data is the union's bytes.
pub fn kind(vt: u16) -> Kind {
    if vt & VT_INDIRECT_MASK != 0 {
        return Kind::Unsupported;
    }
    match vt {
        VT_EMPTY => Kind::Empty,
        VT_LPWSTR => Kind::Text,
        // VT_NULL, I2, I4, R4, R8, CY, DATE, ERROR, BOOL, I1, UI1, UI2, UI4, I8, UI8, INT, UINT,
        // FILETIME.
        1..=7 | 10 | 11 | 16..=23 | 64 => Kind::Scalar,
        _ => Kind::Unsupported,
    }
}

/// The properties whose value is a string (`propkey.h`): `System.AppUserModel.ID` {9F4C2855-9F79-
/// 4B39-A8D0-E1D42DE1D5F3},5 and the relaunch properties ,2 to ,4 of the same format, and
/// `System.Title` {F29F85E0-4FF9-1068-AB91-08002B27B3D9},2. A number set into one of them is read
/// back as its text (sheet rule `any_key`: a `VT_I4` 7 set into `AppUserModel.ID` is `VT_LPWSTR`
/// "7").
pub fn is_text_key(key: &Key) -> bool {
    const APP_USER_MODEL: [u8; 16] =
        [0x55, 0x28, 0x4C, 0x9F, 0x79, 0x9F, 0x39, 0x4B, 0xA8, 0xD0, 0xE1, 0xD4, 0x2D, 0xE1, 0xD5, 0xF3];
    const SUMMARY_INFORMATION: [u8; 16] =
        [0xE0, 0x85, 0x9F, 0xF2, 0xF9, 0x4F, 0x68, 0x10, 0xAB, 0x91, 0x08, 0x00, 0x2B, 0x27, 0xB3, 0xD9];
    let pid = u32::from_le_bytes([key[16], key[17], key[18], key[19]]);
    (key[..16] == APP_USER_MODEL && (2..=5).contains(&pid)) || (key[..16] == SUMMARY_INFORMATION && pid == 2)
}

/// The text of an integer variant (`PROPVARIANT` bytes), when its type is an integer type:
/// `VT_I2` 2, `VT_I4` 3, `VT_I1` 16, `VT_UI1` 17, `VT_UI2` 18, `VT_UI4` 19, `VT_I8` 20, `VT_UI8` 21,
/// `VT_INT` 22, `VT_UINT` 23 (the value's bytes start at offset 8).
pub fn integer_text(variant: &[u8; 24]) -> Option<Vec<u16>> {
    let vt = u16::from_le_bytes([variant[0], variant[1]]);
    let b = &variant[8..16];
    let text = match vt {
        16 => alloc::format!("{}", b[0] as i8),
        17 => alloc::format!("{}", b[0]),
        2 => alloc::format!("{}", i16::from_le_bytes([b[0], b[1]])),
        18 => alloc::format!("{}", u16::from_le_bytes([b[0], b[1]])),
        3 | 22 => alloc::format!("{}", i32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        19 | 23 => alloc::format!("{}", u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        20 => alloc::format!("{}", i64::from_le_bytes(b.try_into().ok()?)),
        21 => alloc::format!("{}", u64::from_le_bytes(b.try_into().ok()?)),
        _ => return None,
    };
    Some(text.encode_utf16().collect())
}

/// A value the store holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// The `PROPVARIANT`'s 24 bytes.
    Scalar([u8; 24]),
    /// A `VT_LPWSTR`'s characters, without the NUL.
    Text(Vec<u16>),
}

/// The properties of one window, in the order they were first set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Store {
    items: Vec<(Key, Value)>,
    /// A value has been set at some time: from then on `GetCount` succeeds, before it it fails
    /// with `E_FAIL` (sheet rule `fresh_store`).
    touched: bool,
}

impl Store {
    pub const fn new() -> Store {
        Store { items: Vec::new(), touched: false }
    }

    /// `IPropertyStore::GetCount`.
    pub fn count(&self) -> u32 {
        self.items.len() as u32
    }

    /// `IPropertyStore::GetCount` as a window's store answers it: `Err` (`E_FAIL`, count 0) while
    /// nothing was ever set.
    pub fn count_checked(&self) -> Result<u32, u32> {
        if self.touched { Ok(self.count()) } else { Err(E_FAIL) }
    }

    /// `IPropertyStore::GetAt`: the key at an index, `None` (`E_INVALIDARG`) past the end.
    pub fn key_at(&self, index: u32) -> Option<Key> {
        self.items.get(index as usize).map(|(k, _)| *k)
    }

    /// `IPropertyStore::GetValue`: the value of a key, `None` (`VT_EMPTY`, `S_OK`) when the store
    /// has none.
    pub fn get(&self, key: &Key) -> Option<&Value> {
        self.items.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// `IPropertyStore::SetValue`: set, replace or (with `None`, a `VT_EMPTY`) remove.
    pub fn set(&mut self, key: &Key, value: Option<Value>) {
        let at = self.items.iter().position(|(k, _)| k == key);
        self.touched |= value.is_some();
        match (at, value) {
            (Some(i), Some(v)) => self.items[i].1 = v,
            (Some(i), None) => {
                self.items.remove(i);
            }
            (None, Some(v)) => self.items.push((*key, v)),
            (None, None) => {}
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> Key {
        let mut k = [0u8; 20];
        k[0] = n;
        k[16] = 5;
        k
    }

    #[test]
    fn variant_types_are_sorted_into_kinds() {
        assert_eq!(kind(VT_EMPTY), Kind::Empty);
        assert_eq!(kind(VT_LPWSTR), Kind::Text);
        for vt in [1u16, 2, 3, 4, 5, 6, 7, 10, 11, 16, 17, 18, 19, 20, 21, 22, 23, 64] {
            assert_eq!(kind(vt), Kind::Scalar, "{vt}");
        }
        // BSTR, DISPATCH, VARIANT, UNKNOWN, LPSTR, BLOB, STREAM, CLSID, vectors, arrays, references.
        for vt in [VT_BSTR, 9, 12, 13, VT_LPSTR, 65, 66, 72, 0x1000 | 3, 0x2000 | 8, 0x4000 | 3] {
            assert_eq!(kind(vt), Kind::Unsupported, "{vt}");
        }
    }

    #[test]
    fn a_store_keeps_values_in_the_order_they_were_set() {
        let mut s = Store::new();
        assert_eq!(s.count(), 0);
        assert_eq!(s.count_checked(), Err(E_FAIL));
        assert_eq!(s.get(&key(1)), None);
        assert_eq!(s.key_at(0), None);
        let id: Vec<u16> = "Chrome.UserData.Default".encode_utf16().collect();
        s.set(&key(1), Some(Value::Text(id.clone())));
        let mut b = [0u8; 24];
        b[0] = 11;
        b[8] = 0xFF;
        s.set(&key(2), Some(Value::Scalar(b)));
        assert_eq!(s.count(), 2);
        assert_eq!(s.key_at(0), Some(key(1)));
        assert_eq!(s.key_at(1), Some(key(2)));
        assert_eq!(s.key_at(2), None);
        assert_eq!(s.get(&key(1)), Some(&Value::Text(id)));
        assert_eq!(s.get(&key(2)), Some(&Value::Scalar(b)));
        // Replacing keeps the place.
        s.set(&key(1), Some(Value::Text(Vec::new())));
        assert_eq!(s.key_at(0), Some(key(1)));
        assert_eq!(s.get(&key(1)), Some(&Value::Text(Vec::new())));
        // VT_EMPTY removes; removing what is not there does nothing.
        s.set(&key(1), None);
        s.set(&key(9), None);
        assert_eq!(s.count(), 1);
        assert_eq!(s.key_at(0), Some(key(2)));
        assert!(!s.is_empty());
        s.set(&key(2), None);
        assert!(s.is_empty());
        // Once a value was set, GetCount succeeds, also when the store is empty again.
        assert_eq!(s.count_checked(), Ok(0));
    }

    #[test]
    fn a_new_store_without_a_value_cannot_count_and_removing_nothing_sets_nothing() {
        let mut s = Store::new();
        s.set(&key(1), None);
        assert_eq!(s.count_checked(), Err(E_FAIL));
        s.set(&key(1), Some(Value::Text(Vec::new())));
        assert_eq!(s.count_checked(), Ok(1));
    }

    #[test]
    fn numbers_set_into_text_keys_read_back_as_text() {
        let mut id = [0u8; 20];
        id[..16].copy_from_slice(&[
            0x55, 0x28, 0x4C, 0x9F, 0x79, 0x9F, 0x39, 0x4B, 0xA8, 0xD0, 0xE1, 0xD4, 0x2D, 0xE1, 0xD5, 0xF3,
        ]);
        id[16] = 5;
        assert!(is_text_key(&id));
        id[16] = 9; // PreventPinning is a boolean
        assert!(!is_text_key(&id));
        assert!(!is_text_key(&key(1)));
        let mut v = [0u8; 24];
        v[0] = 3;
        v[8..12].copy_from_slice(&(-7i32).to_le_bytes());
        assert_eq!(integer_text(&v), Some("-7".encode_utf16().collect()));
        v[0] = 19;
        v[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(integer_text(&v), Some("4294967295".encode_utf16().collect()));
        v[0] = 11;
        assert_eq!(integer_text(&v), None);
    }
}
