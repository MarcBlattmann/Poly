//! Queries and predicates (ID3D11Device.CreateQuery.yaml): the types, their data sizes, Begin / End
//! and GetData. Work is finished when a call returns, so a query that was ended has its data at once.

use crate::hr::*;

/// `D3D11_QUERY` (d3d11.h).
pub mod kind {
    pub const EVENT: u32 = 0;
    pub const OCCLUSION: u32 = 1;
    pub const TIMESTAMP: u32 = 2;
    pub const TIMESTAMP_DISJOINT: u32 = 3;
    pub const PIPELINE_STATISTICS: u32 = 4;
    pub const OCCLUSION_PREDICATE: u32 = 5;
    pub const SO_STATISTICS: u32 = 6;
    pub const SO_OVERFLOW_PREDICATE: u32 = 7;
}

/// `D3D11_QUERY_MISC_PREDICATEHINT`.
pub const PREDICATEHINT: u32 = 1;
/// `D3D11_ASYNC_GETDATA_DONOTFLUSH`.
pub const DONOTFLUSH: u32 = 1;

/// The data size of a query type (`GetDataSize`), `None` for an unknown type.
pub fn data_size(kind: u32) -> Option<u32> {
    Some(match kind {
        0 => 4,
        1 | 2 => 8,
        3 => 16,
        4 => 88,
        5 => 4,
        6 | 8 | 10 | 12 | 14 => 16,
        7 | 9 | 11 | 13 | 15 => 4,
        _ => return None,
    })
}

/// Check `CreateQuery`'s `D3D11_QUERY_DESC`.
pub fn check_query(kind: u32, misc: u32) -> Result<(), HRESULT> {
    if data_size(kind).is_none() {
        return Err(E_INVALIDARG);
    }
    match misc {
        0 => Ok(()),
        PREDICATEHINT if kind == kind::OCCLUSION_PREDICATE => Ok(()),
        _ => Err(E_INVALIDARG),
    }
}

/// Check `CreatePredicate`: only the two predicate types (not the stream variants).
pub fn check_predicate(kind: u32, misc: u32) -> Result<(), HRESULT> {
    if kind != kind::OCCLUSION_PREDICATE && kind != kind::SO_OVERFLOW_PREDICATE {
        return Err(E_INVALIDARG);
    }
    check_query(kind, misc)
}

/// Whether a query of this type is also an `ID3D11Predicate`.
pub fn is_predicate(kind: u32) -> bool {
    kind == kind::OCCLUSION_PREDICATE || kind == kind::SO_OVERFLOW_PREDICATE
}

/// A query's progress.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub kind: u32,
    pub misc: u32,
    /// Begun and not ended since.
    pub open: bool,
    /// Ended at least once (since the last Begin).
    pub ended: bool,
    /// The timestamp taken at End (TIMESTAMP).
    pub stamp: u64,
}

impl Query {
    pub fn new(kind: u32, misc: u32) -> Query {
        Query { kind, misc, ..Default::default() }
    }

    /// `Begin`: ignored by EVENT and TIMESTAMP queries.
    pub fn begin(&mut self) {
        if self.kind != kind::EVENT && self.kind != kind::TIMESTAMP {
            self.open = true;
            self.ended = false;
        }
    }

    /// `End` at time `now`.
    pub fn end(&mut self, now: u64) {
        self.open = false;
        self.ended = true;
        self.stamp = now;
    }

    /// `GetData(pData, DataSize, GetDataFlags)`: the result and the bytes to write (at most
    /// `size`). `freq` is the timestamp frequency.
    pub fn get_data(&self, have_data: bool, size: u32, flags: u32, freq: u64) -> Result<alloc::vec::Vec<u8>, HRESULT> {
        if flags & !DONOTFLUSH != 0 {
            return Err(E_INVALIDARG);
        }
        if self.kind == kind::OCCLUSION_PREDICATE && self.misc & PREDICATEHINT != 0 {
            return Err(E_INVALIDARG);
        }
        let full = data_size(self.kind).unwrap_or(0);
        if size != 0 && size != full || size == full && !have_data && size != 0 {
            return Err(E_INVALIDARG);
        }
        if self.open || !self.ended {
            return Err(DXGI_ERROR_INVALID_CALL);
        }
        if size == 0 {
            return Ok(alloc::vec::Vec::new());
        }
        let mut v = alloc::vec![0u8; full as usize];
        match self.kind {
            kind::EVENT => v[0] = 1,
            kind::TIMESTAMP => v.copy_from_slice(&self.stamp.max(1).to_le_bytes()),
            // D3D11_QUERY_DATA_TIMESTAMP_DISJOINT: Frequency, Disjoint (the padding is not written).
            kind::TIMESTAMP_DISJOINT => {
                v[..8].copy_from_slice(&freq.to_le_bytes());
                v.truncate(12);
            }
            _ => {}
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_and_flags() {
        for k in 0..16 {
            assert!(check_query(k, 0).is_ok());
        }
        for k in [16, 17, 100, 0x4000_0000, u32::MAX] {
            assert_eq!(check_query(k, 0), Err(E_INVALIDARG));
        }
        assert!(check_query(5, 1).is_ok());
        assert_eq!(check_query(0, 1), Err(E_INVALIDARG));
        assert_eq!(check_query(5, 2), Err(E_INVALIDARG));
        assert!(check_predicate(7, 0).is_ok());
        assert_eq!(check_predicate(9, 0), Err(E_INVALIDARG));
        assert_eq!(data_size(4), Some(88));
    }

    #[test]
    fn event_and_get_data() {
        let mut q = Query::new(kind::EVENT, 0);
        assert_eq!(q.get_data(true, 4, 0, 1), Err(DXGI_ERROR_INVALID_CALL));
        q.begin();
        q.end(5);
        assert_eq!(q.get_data(true, 4, 0, 1), Ok(alloc::vec![1, 0, 0, 0]));
        assert_eq!(q.get_data(false, 0, 0, 1), Ok(alloc::vec![]));
        for bad in [1, 2, 3, 5, 8] {
            assert_eq!(q.get_data(true, bad, 0, 1), Err(E_INVALIDARG));
        }
        assert_eq!(q.get_data(false, 4, 0, 1), Err(E_INVALIDARG));
        assert_eq!(q.get_data(true, 4, 2, 1), Err(E_INVALIDARG));
        let mut d = Query::new(kind::TIMESTAMP_DISJOINT, 0);
        d.begin();
        assert_eq!(d.get_data(true, 16, 0, 1), Err(DXGI_ERROR_INVALID_CALL));
        d.end(1);
        assert_eq!(d.get_data(true, 16, 0, 10_000_000).unwrap().len(), 12);
    }
}
