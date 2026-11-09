//! `WSADuplicateSocketW` and `WSASocketW(FROM_PROTOCOL_INFO)` (Learn:
//! "WSADuplicateSocketW", "WSAPROTOCOL_INFOW"; ADR 0155). A socket is
//! handed to another process in two steps: the owner duplicates the
//! endpoint handle into the target process and fills a `WSAPROTOCOL_INFOW`
//! describing it, the target passes that record to `WSASocketW` and gets
//! the socket. The record is 628 bytes (`winsock2.h`); Windows keeps its
//! own provider data in it. Poly's record carries Poly's provider id, the
//! socket's address family, type and protocol (so the target can check them) and the handle's
//! value in the target process.
//!
//! `WSAPROTOCOL_INFOW` (x64): `dwServiceFlags1..4` 0, `dwProviderFlags` 16,
//! `ProviderId` 20, `dwCatalogEntryId` 36, `ProtocolChain` 40 (`ChainLen`
//! 40, `ChainEntries[7]` 44), `iVersion` 72, `iAddressFamily` 76,
//! `iMaxSockAddr` 80, `iMinSockAddr` 84, `iSocketType` 88, `iProtocol` 92,
//! `iProtocolMaxOffset` 96, `iNetworkByteOrder` 100, `iSecurityScheme` 104,
//! `dwMessageSize` 108, `dwProviderReserved` 112, `szProtocol[256]` 116;
//! 628 bytes.

use crate::addr;

pub const PROTOCOL_INFO_SIZE: usize = 628;
/// `FROM_PROTOCOL_INFO` (`winsock2.h`): `WSASocketW`'s family, type or protocol -1 takes them from the record.
pub const FROM_PROTOCOL_INFO: i32 = -1;

/// Poly's provider id in the record: `{504F4C59-4146-4400-8000-000000000001}` ("POLY", "AFD").
pub const PROVIDER_ID: [u8; 16] =
    [0x59, 0x4C, 0x4F, 0x50, 0x46, 0x41, 0x00, 0x44, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01];

/// What the record carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Duplicate {
    /// The socket's handle value in the target process.
    pub handle: u64,
    pub family: i32,
    pub ty: i32,
    pub protocol: i32,
}

const PROVIDER_ID_AT: usize = 20;
const CHAIN_LEN_AT: usize = 40;
const FAMILY_AT: usize = 76;
const MAX_SOCKADDR_AT: usize = 80;
const MIN_SOCKADDR_AT: usize = 84;
const TYPE_AT: usize = 88;
const PROTOCOL_AT: usize = 92;
const RESERVED_AT: usize = 112;
/// The handle's low and high halves: `ChainEntries[0]` and `[1]`.
const HANDLE_LOW_AT: usize = 44;
const HANDLE_HIGH_AT: usize = 48;
const PROTOCOL_NAME_AT: usize = 116;

/// The record for `d`.
pub fn encode(d: &Duplicate) -> [u8; PROTOCOL_INFO_SIZE] {
    let mut b = [0u8; PROTOCOL_INFO_SIZE];
    b[PROVIDER_ID_AT..PROVIDER_ID_AT + 16].copy_from_slice(&PROVIDER_ID);
    // One protocol in the chain (a base provider: ChainLen 1).
    b[CHAIN_LEN_AT..CHAIN_LEN_AT + 4].copy_from_slice(&1i32.to_le_bytes());
    b[HANDLE_LOW_AT..HANDLE_LOW_AT + 4].copy_from_slice(&(d.handle as u32).to_le_bytes());
    b[HANDLE_HIGH_AT..HANDLE_HIGH_AT + 4].copy_from_slice(&((d.handle >> 32) as u32).to_le_bytes());
    b[FAMILY_AT..FAMILY_AT + 4].copy_from_slice(&d.family.to_le_bytes());
    let (max, min) = if d.family == crate::consts::AF_INET6 {
        (addr::SOCKADDR_IN6_SIZE as i32, addr::SOCKADDR_IN6_SIZE as i32)
    } else {
        (addr::SOCKADDR_IN_SIZE as i32, addr::SOCKADDR_IN_SIZE as i32)
    };
    b[MAX_SOCKADDR_AT..MAX_SOCKADDR_AT + 4].copy_from_slice(&max.to_le_bytes());
    b[MIN_SOCKADDR_AT..MIN_SOCKADDR_AT + 4].copy_from_slice(&min.to_le_bytes());
    b[TYPE_AT..TYPE_AT + 4].copy_from_slice(&d.ty.to_le_bytes());
    b[PROTOCOL_AT..PROTOCOL_AT + 4].copy_from_slice(&d.protocol.to_le_bytes());
    b[RESERVED_AT..RESERVED_AT + 4].copy_from_slice(&1u32.to_le_bytes());
    let name = b"Poly AFD";
    for (i, c) in name.iter().enumerate() {
        b[PROTOCOL_NAME_AT + 2 * i] = *c;
    }
    b
}

/// The socket a record names (`None` if it is not Poly's, or is too short).
pub fn decode(b: &[u8]) -> Option<Duplicate> {
    if b.len() < PROTOCOL_INFO_SIZE || b[PROVIDER_ID_AT..PROVIDER_ID_AT + 16] != PROVIDER_ID {
        return None;
    }
    let i32_at = |at: usize| i32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    let low = u64::from(u32::from_le_bytes(b[HANDLE_LOW_AT..HANDLE_LOW_AT + 4].try_into().unwrap()));
    let high = u64::from(u32::from_le_bytes(b[HANDLE_HIGH_AT..HANDLE_HIGH_AT + 4].try_into().unwrap()));
    Some(Duplicate {
        handle: low | high << 32,
        family: i32_at(FAMILY_AT),
        ty: i32_at(TYPE_AT),
        protocol: i32_at(PROTOCOL_AT),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_round_trips() {
        let d = Duplicate { handle: 0x1_0000_0ABC, family: 2, ty: 1, protocol: 6 };
        let b = encode(&d);
        assert_eq!(b.len(), 628);
        assert_eq!(decode(&b), Some(d));
        // Learn's fields: version-independent ones a reader checks.
        assert_eq!(i32::from_le_bytes(b[40..44].try_into().unwrap()), 1, "ChainLen");
        assert_eq!(i32::from_le_bytes(b[80..84].try_into().unwrap()), 16);
        let d6 = Duplicate { handle: 8, family: 23, ty: 2, protocol: 17 };
        assert_eq!(i32::from_le_bytes(encode(&d6)[80..84].try_into().unwrap()), 28);
    }

    #[test]
    fn foreign_or_short_records_are_refused() {
        let mut b = encode(&Duplicate { handle: 4, family: 2, ty: 1, protocol: 6 });
        assert_eq!(decode(&b[..100]), None);
        b[20] ^= 0xFF;
        assert_eq!(decode(&b), None);
        assert_eq!(decode(&[0u8; 628]), None);
    }
}
