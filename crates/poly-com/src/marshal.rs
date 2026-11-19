//! Standard COM marshalling data (`OBJREF`, `[MS-DCOM]` 2.2.18) and the per-process table of
//! marshalled object references (ADR 0220, B2b).
//!
//! Host-tested logic shared by `ole32`:
//! - Context validation (`MSHCTX_*`, `wtypes.h`).
//! - Encoding and two-phase decoding of `OBJREF_STANDARD` (68 bytes: 24-byte header + 40-byte
//!   `STDOBJREF` + 4-byte empty `DUALSTRINGARRAY`) and `OBJREF_CUSTOM` (for `MSHCTX_INPROC`).
//! - Converting between `(pid, oxid)` and Poly's exporter pipe name.
//! - Tracking live `NORMAL`, `TABLESTRONG` and `TABLEWEAK` marshal entries.
//!
//! Sources: `[MS-DCOM]` 2.2.18, `objidl.h`, `wtypes.h`, `winerror.h`, and the spec sheets
//! `CoMarshalInterface`, `CoUnmarshalInterface`, `CoReleaseMarshalData`, `CoGetMarshalSizeMax`,
//! `CoGetStandardMarshal`.

use crate::git::Owner;
use crate::guid::Guid;
use crate::hresult::{E_INVALIDARG, E_UNEXPECTED, Hresult, RPC_E_INVALID_OBJREF};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// `OBJREF` signature: ASCII `"MEOW"` in little-endian (`[MS-DCOM]` 2.2.18).
pub const OBJREF_SIGNATURE: u32 = 0x574F_454D;
/// `OBJREF_STANDARD` (`[MS-DCOM]` 2.2.18).
pub const OBJREF_STANDARD: u32 = 1;
/// `OBJREF_CUSTOM` (`[MS-DCOM]` 2.2.18).
pub const OBJREF_CUSTOM: u32 = 4;

/// `MSHCTX_*` (`wtypesbase.h`).
pub const MSHCTX_LOCAL: u32 = 0;
pub const MSHCTX_NOSHAREDMEM: u32 = 1;
pub const MSHCTX_DIFFERENTMACHINE: u32 = 2;
pub const MSHCTX_INPROC: u32 = 4;

/// `MSHLFLAGS_*` (`wtypesbase.h`).
pub const MSHLFLAGS_NORMAL: u32 = 0;
pub const MSHLFLAGS_TABLESTRONG: u32 = 1;
pub const MSHLFLAGS_TABLEWEAK: u32 = 2;
pub const MSHLFLAGS_NOPING: u32 = 4;

/// `CLSID_StdMarshal` (`objbase.h`): `{00000017-0000-0000-C000-000000000046}`.
pub const CLSID_STD_MARSHAL: Guid =
    Guid { data1: 0x17, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };

/// Size of the fixed `OBJREF` prefix (`signature`, `flags`, `iid`).
pub const OBJREF_HEADER_LEN: usize = 24;
/// Size of Poly's `OBJREF_STANDARD`: 24-byte header + 40-byte `STDOBJREF` + 4-byte empty
/// `DUALSTRINGARRAY`.
pub const OBJREF_STANDARD_LEN: usize = 68;
/// Size of Poly's `OBJREF_CUSTOM` (`MSHCTX_INPROC`): 24-byte header + 24-byte custom header +
/// 24-byte payload.
pub const OBJREF_CUSTOM_LEN: usize = 72;

/// Pipe prefix of Poly's object exporters (ADR 0220).
const PIPE_PREFIX: &str = r"\\.\pipe\poly-com-ox-";

/// Check `dwDestContext` (sheets `CoMarshalInterface` and `CoGetMarshalSizeMax`, rule
/// `contexts_and_flags`): 0..=4 succeed, 5 is `E_UNEXPECTED`, 6 and above are `E_INVALIDARG`.
pub fn check_context(ctx: u32) -> Result<(), Hresult> {
    match ctx {
        0..=4 => Ok(()),
        5 => Err(E_UNEXPECTED),
        _ => Err(E_INVALIDARG),
    }
}

/// The OBJREF `flags` word for a validated `dwDestContext`.
pub fn objref_flags_for(ctx: u32) -> u32 {
    if ctx == MSHCTX_INPROC { OBJREF_CUSTOM } else { OBJREF_STANDARD }
}

/// Upper bound returned by `CoGetMarshalSizeMax` for a validated context.
pub fn marshal_size_max(ctx: u32) -> u32 {
    if ctx == MSHCTX_INPROC { OBJREF_CUSTOM_LEN as u32 } else { OBJREF_STANDARD_LEN as u32 }
}

/// The exporter pipe name for `(pid, oxid)`.
pub fn endpoint_for(pid: u32, oxid: u64) -> String {
    format!("{PIPE_PREFIX}{pid}-{oxid:016x}")
}

/// Parse `(pid, oxid)` back from an exporter pipe name made by [`endpoint_for`].
pub fn parse_endpoint(endpoint: &str) -> Option<(u32, u64)> {
    let rest = endpoint.strip_prefix(PIPE_PREFIX)?;
    let (p, o) = rest.split_once('-')?;
    let pid = p.parse::<u32>().ok()?;
    let oxid = u64::from_str_radix(o, 16).ok()?;
    Some((pid, oxid))
}

/// Lifetime semantics of a marshal packet (from the low two bits of `mshlflags`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarshalKind {
    Normal,
    TableStrong,
    TableWeak,
}

impl MarshalKind {
    pub fn from_flags(mshlflags: u32) -> MarshalKind {
        match mshlflags & 3 {
            MSHLFLAGS_TABLESTRONG | 3 => MarshalKind::TableStrong,
            MSHLFLAGS_TABLEWEAK => MarshalKind::TableWeak,
            _ => MarshalKind::Normal,
        }
    }
}

/// The parsed 24-byte `OBJREF` header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjRefHeader {
    pub flags: u32,
    pub iid: Guid,
}

/// A decoded standard or in-process `OBJREF`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjRefData {
    pub flags: u32,
    pub iid: Guid,
    pub mshlflags: u32,
    pub pid: u32,
    pub oxid: u64,
    pub oid: u64,
    pub token: u64,
}

fn push_guid(out: &mut Vec<u8>, g: &Guid) {
    out.extend_from_slice(&g.data1.to_le_bytes());
    out.extend_from_slice(&g.data2.to_le_bytes());
    out.extend_from_slice(&g.data3.to_le_bytes());
    out.extend_from_slice(&g.data4);
}

fn read_guid(b: &[u8]) -> Guid {
    Guid {
        data1: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        data2: u16::from_le_bytes([b[4], b[5]]),
        data3: u16::from_le_bytes([b[6], b[7]]),
        data4: [b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]],
    }
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn le64(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

impl ObjRefData {
    /// Encode as an `OBJREF_STANDARD` (68 bytes) or `OBJREF_CUSTOM` (72 bytes).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(if self.flags == OBJREF_CUSTOM {
            OBJREF_CUSTOM_LEN
        } else {
            OBJREF_STANDARD_LEN
        });
        out.extend_from_slice(&OBJREF_SIGNATURE.to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
        push_guid(&mut out, &self.iid);
        if self.flags == OBJREF_CUSTOM {
            // OBJREF_CUSTOM: clsid (16) + cbExtension (4) + size (4) + payload (24).
            push_guid(&mut out, &CLSID_STD_MARSHAL);
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&24u32.to_le_bytes());
            out.extend_from_slice(&self.pid.to_le_bytes());
            out.extend_from_slice(&self.mshlflags.to_le_bytes());
            out.extend_from_slice(&self.oid.to_le_bytes());
            out.extend_from_slice(&self.token.to_le_bytes());
        } else {
            // STDOBJREF (40 bytes): flags (4), cPublicRefs (4), oxid (8), oid (8), ipid (16).
            let c_public_refs: u32 = if MarshalKind::from_flags(self.mshlflags) == MarshalKind::Normal { 1 } else { 0 };
            out.extend_from_slice(&self.mshlflags.to_le_bytes());
            out.extend_from_slice(&c_public_refs.to_le_bytes());
            out.extend_from_slice(&self.oxid.to_le_bytes());
            out.extend_from_slice(&self.oid.to_le_bytes());
            // IPID carries (pid, mshlflags, token).
            out.extend_from_slice(&self.pid.to_le_bytes());
            out.extend_from_slice(&self.mshlflags.to_le_bytes());
            out.extend_from_slice(&self.token.to_le_bytes());
            // Empty DUALSTRINGARRAY: wNumEntries = 0, wSecurityOffset = 0.
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        out
    }
}

/// Phase 1 of decoding: validate the 24-byte `OBJREF` header.
///
/// Reading only the 24-byte header before checking the signature matches `CoReleaseMarshalData`'s
/// observed position of 24 on damaged data (`relmarshal_basic`).
pub fn decode_header(head: &[u8; OBJREF_HEADER_LEN]) -> Result<ObjRefHeader, Hresult> {
    if le32(&head[0..4]) != OBJREF_SIGNATURE {
        return Err(RPC_E_INVALID_OBJREF);
    }
    let flags = le32(&head[4..8]);
    if flags != OBJREF_STANDARD && flags != OBJREF_CUSTOM {
        return Err(RPC_E_INVALID_OBJREF);
    }
    Ok(ObjRefHeader { flags, iid: read_guid(&head[8..24]) })
}

/// Phase 2 for `OBJREF_STANDARD`: decode the 44 bytes after the header (`STDOBJREF` + the 4-byte
/// `DUALSTRINGARRAY` header), returning the decoded data and any trailing string-array bytes to
/// skip (`wNumEntries * 2`).
pub fn decode_standard_body(iid: Guid, body: &[u8; 44]) -> (ObjRefData, usize) {
    let mshlflags = le32(&body[0..4]);
    let oxid = le64(&body[8..16]);
    let oid = le64(&body[16..24]);
    let pid = le32(&body[24..28]);
    let token = le64(&body[32..40]);
    let num_entries = u16::from_le_bytes([body[40], body[41]]) as usize;
    let data = ObjRefData { flags: OBJREF_STANDARD, iid, mshlflags, pid, oxid, oid, token };
    (data, num_entries * 2)
}

/// Phase 2 for `OBJREF_CUSTOM`: validate the 24-byte custom header (`clsid`, `cbExtension`, `size`)
/// and return `(clsid, payload_size)`.
pub fn decode_custom_head(head: &[u8; 24]) -> Result<(Guid, usize), Hresult> {
    let clsid = read_guid(&head[0..16]);
    let size = le32(&head[20..24]) as usize;
    Ok((clsid, size))
}

/// Phase 3 for Poly's `CLSID_StdMarshal` custom payload (24 bytes).
pub fn decode_custom_payload(iid: Guid, payload: &[u8]) -> Result<ObjRefData, Hresult> {
    if payload.len() < 24 {
        return Err(RPC_E_INVALID_OBJREF);
    }
    let pid = le32(&payload[0..4]);
    let mshlflags = le32(&payload[4..8]);
    let oid = le64(&payload[8..16]);
    let token = le64(&payload[16..24]);
    Ok(ObjRefData { flags: OBJREF_CUSTOM, iid, mshlflags, pid, oxid: 0, oid, token })
}

/// One live marshal entry in this process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarshalEntry {
    pub token: u64,
    /// Object id in this process's exporter (0 for a marshalled remote proxy).
    pub oid: u64,
    /// Canonical `IUnknown` of a local object, or the proxy's `IUnknown` when `is_remote_proxy`.
    pub object: usize,
    pub owner: Owner,
    pub kind: MarshalKind,
    pub is_remote_proxy: bool,
}

/// Per-process table of live marshal tokens (`NORMAL`, `TABLESTRONG`, `TABLEWEAK`).
#[derive(Debug)]
pub struct MarshalTable {
    next_token: u64,
    entries: Vec<MarshalEntry>,
}

impl Default for MarshalTable {
    fn default() -> Self {
        Self::new()
    }
}

impl MarshalTable {
    pub const fn new() -> MarshalTable {
        MarshalTable { next_token: 1, entries: Vec::new() }
    }

    /// Record a new marshal entry and return its unique non-zero token.
    pub fn insert(
        &mut self,
        oid: u64,
        object: usize,
        owner: Owner,
        kind: MarshalKind,
        is_remote_proxy: bool,
    ) -> u64 {
        let token = self.next_token;
        self.next_token = self.next_token.wrapping_add(1).max(1);
        self.entries.push(MarshalEntry { token, oid, object, owner, kind, is_remote_proxy });
        token
    }

    /// Look up `token` for unmarshalling.
    ///
    /// Returns `(entry, consumed)`: a `Normal` entry is removed so a second unmarshal fails with
    /// `CO_E_OBJNOTCONNECTED`; table entries stay until [`release_data`](Self::release_data).
    pub fn unmarshal(&mut self, token: u64) -> Option<(MarshalEntry, bool)> {
        let i = self.entries.iter().position(|e| e.token == token)?;
        if self.entries[i].kind == MarshalKind::Normal {
            Some((self.entries.remove(i), true))
        } else {
            Some((self.entries[i], false))
        }
    }

    /// Whether `token` is a live entry.
    pub fn contains(&self, token: u64) -> bool {
        self.entries.iter().any(|e| e.token == token)
    }

    /// Remove `token` for `CoReleaseMarshalData`, returning the entry whose reference must be
    /// released (or `None` if already consumed or released).
    pub fn release_data(&mut self, token: u64) -> Option<MarshalEntry> {
        let i = self.entries.iter().position(|e| e.token == token)?;
        Some(self.entries.remove(i))
    }

    /// Remove all entries belonging to `owner` (apartment ended).
    pub fn remove_apartment(&mut self, owner: Owner) -> Vec<MarshalEntry> {
        let (gone, keep) = self.entries.drain(..).partition(|e| e.owner == owner && !e.is_remote_proxy);
        self.entries = keep;
        gone
    }

    /// Remove all entries for local object `identity` (`CoDisconnectObject`).
    pub fn remove_object(&mut self, identity: usize) -> Vec<MarshalEntry> {
        let (gone, keep) = self.entries.drain(..).partition(|e| e.object == identity && !e.is_remote_proxy);
        self.entries = keep;
        gone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IID: Guid = Guid { data1: 0xB2B0_0001, data2: 0x1234, data3: 0x4000, data4: [0x80, 0, 1, 2, 3, 4, 5, 6] };

    #[test]
    fn context_rules() {
        for c in 0..=4 {
            assert_eq!(check_context(c), Ok(()));
        }
        assert_eq!(check_context(5), Err(E_UNEXPECTED));
        assert_eq!(check_context(6), Err(E_INVALIDARG));
        assert_eq!(check_context(0x8000_0000), Err(E_INVALIDARG));
        assert_eq!(objref_flags_for(MSHCTX_LOCAL), OBJREF_STANDARD);
        assert_eq!(objref_flags_for(MSHCTX_NOSHAREDMEM), OBJREF_STANDARD);
        assert_eq!(objref_flags_for(MSHCTX_INPROC), OBJREF_CUSTOM);
    }

    #[test]
    fn standard_objref_is_68_bytes_and_round_trips() {
        let d = ObjRefData {
            flags: OBJREF_STANDARD,
            iid: IID,
            mshlflags: MSHLFLAGS_TABLESTRONG,
            pid: 4242,
            oxid: 0x0123_4567_89AB_CDEF,
            oid: 99,
            token: 777,
        };
        let bytes = d.encode();
        assert_eq!(bytes.len(), 68);
        assert_eq!(&bytes[0..4], b"MEOW");
        assert!(marshal_size_max(MSHCTX_LOCAL) as usize >= bytes.len());
        let head = decode_header(bytes[0..24].try_into().unwrap()).unwrap();
        assert_eq!(head.flags, OBJREF_STANDARD);
        assert_eq!(head.iid, IID);
        let (back, extra) = decode_standard_body(head.iid, bytes[24..68].try_into().unwrap());
        assert_eq!(extra, 0);
        assert_eq!(back, d);
    }

    #[test]
    fn custom_inproc_objref_round_trips() {
        let d = ObjRefData {
            flags: OBJREF_CUSTOM,
            iid: IID,
            mshlflags: MSHLFLAGS_NORMAL,
            pid: 123,
            oxid: 0,
            oid: 5,
            token: 11,
        };
        let bytes = d.encode();
        assert_eq!(bytes.len(), 72);
        assert!(marshal_size_max(MSHCTX_INPROC) as usize >= bytes.len());
        let head = decode_header(bytes[0..24].try_into().unwrap()).unwrap();
        assert_eq!(head.flags, OBJREF_CUSTOM);
        let (clsid, size) = decode_custom_head(bytes[24..48].try_into().unwrap()).unwrap();
        assert_eq!(clsid, CLSID_STD_MARSHAL);
        assert_eq!(size, 24);
        let back = decode_custom_payload(head.iid, &bytes[48..72]).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn bad_header_is_invalid_objref() {
        let bad = [7u8; 24];
        assert_eq!(decode_header(&bad), Err(RPC_E_INVALID_OBJREF));
        let mut bad_flags = [0u8; 24];
        bad_flags[0..4].copy_from_slice(b"MEOW");
        bad_flags[4..8].copy_from_slice(&9u32.to_le_bytes());
        assert_eq!(decode_header(&bad_flags), Err(RPC_E_INVALID_OBJREF));
    }

    #[test]
    fn endpoint_round_trip() {
        let ep = endpoint_for(1337, 0x0000_0001_2345_6789);
        assert_eq!(ep, r"\\.\pipe\poly-com-ox-1337-0000000123456789");
        assert_eq!(parse_endpoint(&ep), Some((1337, 0x0000_0001_2345_6789)));
        assert_eq!(parse_endpoint("bad"), None);
    }

    #[test]
    fn marshal_table_lifetimes() {
        let mut t = MarshalTable::new();
        let tok_n = t.insert(1, 0x1000, Owner::Mta, MarshalKind::Normal, false);
        let tok_s = t.insert(1, 0x1000, Owner::Mta, MarshalKind::TableStrong, false);
        let tok_w = t.insert(2, 0x2000, Owner::Sta(5), MarshalKind::TableWeak, false);

        // Normal is consumed on first unmarshal.
        let (e, consumed) = t.unmarshal(tok_n).unwrap();
        assert!(consumed);
        assert_eq!(e.object, 0x1000);
        assert_eq!(t.unmarshal(tok_n), None);
        assert_eq!(t.release_data(tok_n), None);

        // TableStrong survives multiple unmarshals until release_data.
        for _ in 0..3 {
            let (e, consumed) = t.unmarshal(tok_s).unwrap();
            assert!(!consumed);
            assert_eq!(e.object, 0x1000);
        }
        assert!(t.release_data(tok_s).is_some());
        assert_eq!(t.unmarshal(tok_s), None);
        assert_eq!(t.release_data(tok_s), None);

        // Apartment cleanup removes STA entries.
        let gone = t.remove_apartment(Owner::Sta(5));
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].token, tok_w);
        assert_eq!(t.unmarshal(tok_w), None);
    }
}
