//! Private data of a D3D11 or DXGI object (ID3D11Device.GetCreationFlags.yaml `private_data`,
//! IDXGIDevice.GetAdapter.yaml `private_data_*`): blobs and interface pointers by GUID.

use crate::hr::*;
use alloc::vec::Vec;

/// What is stored under one GUID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Bytes(Vec<u8>),
    /// An interface pointer (the owner holds a reference to it).
    Interface(usize),
}

/// The private data of one object. GUIDs as their 16 bytes.
#[derive(Default, Debug)]
pub struct Store {
    entries: Vec<([u8; 16], Value)>,
}

/// What `GetPrivateData` gives.
#[derive(Debug, PartialEq, Eq)]
pub enum Got<'a> {
    /// `hr`, the size to report, and the bytes to copy (None: copy nothing).
    Data { hr: HRESULT, size: u32, bytes: Option<&'a [u8]> },
    /// An interface: S_OK, size 8, the pointer (the caller adds a reference).
    Interface(usize),
}

impl Store {
    pub const fn new() -> Store {
        Store { entries: Vec::new() }
    }

    fn find(&self, g: &[u8; 16]) -> Option<usize> {
        self.entries.iter().position(|(k, _)| k == g)
    }

    /// `SetPrivateData(guid, size, data)`: replaces the entry; size 0 removes it; NULL data with a
    /// size is S_FALSE and changes nothing. Returns the result and a replaced interface pointer the
    /// caller must release.
    pub fn set(&mut self, g: [u8; 16], data: Option<&[u8]>, size: u32) -> (HRESULT, Option<usize>) {
        let old = self.find(&g);
        if size == 0 {
            let gone = old.map(|i| self.entries.remove(i).1);
            return (S_OK, iface(gone));
        }
        let Some(bytes) = data else { return (S_FALSE, None) };
        let v = Value::Bytes(bytes.to_vec());
        match old {
            Some(i) => {
                let prev = core::mem::replace(&mut self.entries[i].1, v);
                (S_OK, iface(Some(prev)))
            }
            None => {
                self.entries.push((g, v));
                (S_OK, None)
            }
        }
    }

    /// `SetPrivateDataInterface(guid, p)`: stores `p` (the caller added a reference) or removes the
    /// entry for NULL; returns a replaced interface the caller releases.
    pub fn set_interface(&mut self, g: [u8; 16], p: usize) -> Option<usize> {
        // NULL is stored too: the entry then holds no interface (IDXGIDevice.GetAdapter.yaml
        // `private_data_interface`: a following GetPrivateData is S_OK).
        let old = self.find(&g);
        match old {
            Some(i) => iface(Some(core::mem::replace(&mut self.entries[i].1, Value::Interface(p)))),
            None => {
                self.entries.push((g, Value::Interface(p)));
                None
            }
        }
    }

    /// `GetPrivateData(guid, &size, buffer)` with the caller's `size` and whether it gave a buffer.
    pub fn get(&self, g: &[u8; 16], size: u32, buffer: bool) -> Got<'_> {
        let Some(i) = self.find(g) else { return Got::Data { hr: DXGI_ERROR_NOT_FOUND, size: 0, bytes: None } };
        match &self.entries[i].1 {
            Value::Interface(p) => {
                if buffer && size >= 8 {
                    Got::Interface(*p)
                } else if !buffer {
                    Got::Data { hr: S_OK, size: 8, bytes: None }
                } else {
                    Got::Data { hr: DXGI_ERROR_MORE_DATA, size: 8, bytes: None }
                }
            }
            Value::Bytes(b) => {
                let n = b.len() as u32;
                if !buffer {
                    Got::Data { hr: S_OK, size: n, bytes: None }
                } else if size < n {
                    Got::Data { hr: DXGI_ERROR_MORE_DATA, size: n, bytes: None }
                } else {
                    Got::Data { hr: S_OK, size: n, bytes: Some(b) }
                }
            }
        }
    }

    /// Every interface pointer held (to release when the object goes).
    pub fn interfaces(&self) -> impl Iterator<Item = usize> + '_ {
        self.entries.iter().filter_map(|(_, v)| match v {
            Value::Interface(p) if *p != 0 => Some(*p),
            _ => None,
        })
    }
}

fn iface(v: Option<Value>) -> Option<usize> {
    match v {
        Some(Value::Interface(p)) if p != 0 => Some(p),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blobs() {
        let mut s = Store::new();
        let g = [1u8; 16];
        assert_eq!(s.get(&g, 8, true), Got::Data { hr: DXGI_ERROR_NOT_FOUND, size: 0, bytes: None });
        assert_eq!(s.set(g, Some(&[1, 2, 3, 4, 5]), 5).0, S_OK);
        assert_eq!(s.get(&g, 0, false), Got::Data { hr: S_OK, size: 5, bytes: None });
        assert_eq!(s.get(&g, 3, true), Got::Data { hr: DXGI_ERROR_MORE_DATA, size: 5, bytes: None });
        assert_eq!(s.get(&g, 8, true), Got::Data { hr: S_OK, size: 5, bytes: Some(&[1, 2, 3, 4, 5][..]) });
        assert_eq!(s.set(g, None, 4).0, S_FALSE);
        assert_eq!(s.set(g, None, 0).0, S_OK);
        assert_eq!(s.get(&g, 8, true), Got::Data { hr: DXGI_ERROR_NOT_FOUND, size: 0, bytes: None });
    }

    #[test]
    fn interfaces() {
        let mut s = Store::new();
        let g = [2u8; 16];
        assert_eq!(s.set_interface(g, 0x1000), None);
        assert_eq!(s.get(&g, 8, true), Got::Interface(0x1000));
        assert_eq!(s.get(&g, 4, true), Got::Data { hr: DXGI_ERROR_MORE_DATA, size: 8, bytes: None });
        assert_eq!(s.set_interface(g, 0), Some(0x1000));
        assert_eq!(s.get(&g, 8, true), Got::Interface(0));
    }
}
