//! `GetIfTable2`: `MIB_IF_TABLE2` of `MIB_IF_ROW2` (x64, `netioapi.h`).
//! The table is `ULONG NumEntries`, 4 bytes of padding, then the rows
//! ([`ROW_SIZE`] = 1352 bytes each); the API allocates it and
//! `FreeMibTable` frees it.
//!
//! Row layout (x64): InterfaceLuid 0, InterfaceIndex 8, InterfaceGuid 12,
//! Alias[257] 28, Description[257] 542, PhysicalAddressLength 1056,
//! PhysicalAddress[32] 1060, PermanentPhysicalAddress[32] 1092, Mtu 1124,
//! Type 1128, TunnelType 1132, MediaType 1136, PhysicalMediumType 1140,
//! AccessType 1144, DirectionType 1148, InterfaceAndOperStatusFlags 1152,
//! OperStatus 1156, AdminStatus 1160, MediaConnectState 1164, NetworkGuid
//! 1168, ConnectionType 1184, TransmitLinkSpeed 1192, ReceiveLinkSpeed 1200,
//! then sixteen `ULONG64` counters from InOctets 1208 to OutQLen 1344.

use crate::{IfInfo, Kind, Out};
use alloc::vec::Vec;

pub const ROW_SIZE: usize = 1352;
pub const TABLE_HEADER: usize = 8;

/// `NdisMedium802_3`, `NdisMediumLoopback` (`ntddndis.h`).
const MEDIUM_802_3: u32 = 0;
const MEDIUM_LOOPBACK: u32 = 17;
/// `NdisPhysicalMedium802_3`, `NdisPhysicalMediumUnspecified`.
const PHYSICAL_802_3: u32 = 14;
const PHYSICAL_UNSPECIFIED: u32 = 0;
/// `NET_IF_ACCESS_LOOPBACK`, `NET_IF_ACCESS_BROADCAST`; `NET_IF_DIRECTION_SENDRECEIVE`.
const ACCESS_LOOPBACK: u32 = 1;
const ACCESS_BROADCAST: u32 = 2;
/// `IfOperStatusUp`, `IfOperStatusDown`; `NET_IF_ADMIN_STATUS_UP`;
/// `MediaConnectStateConnected`, `MediaConnectStateDisconnected`.
const OPER_UP: u32 = 1;
const OPER_DOWN: u32 = 2;
const ADMIN_UP: u32 = 1;
const MEDIA_CONNECTED: u32 = 1;
const MEDIA_DISCONNECTED: u32 = 2;
/// `InterfaceAndOperStatusFlags`: `HardwareInterface` (bit 0), `ConnectorPresent` (bit 2),
/// `NotMediaConnected` (bit 4).
const HARDWARE_INTERFACE: u8 = 1;
const CONNECTOR_PRESENT: u8 = 4;
const NOT_MEDIA_CONNECTED: u8 = 16;

fn put_wide_fixed(o: &mut Out, at: usize, s: &str, max_units: usize) {
    for (i, c) in s.encode_utf16().take(max_units - 1).enumerate() {
        o.put_u16(at + i * 2, c);
    }
}

/// The `MIB_IF_TABLE2` bytes for `ifs`.
pub fn table(ifs: &[IfInfo]) -> Vec<u8> {
    let mut o = Out::new(0);
    o.alloc(TABLE_HEADER + ifs.len() * ROW_SIZE);
    o.put_u32(0, ifs.len() as u32);
    for (i, f) in ifs.iter().enumerate() {
        let r = TABLE_HEADER + i * ROW_SIZE;
        o.put_u64(r, f.luid());
        o.put_u32(r + 8, f.index);
        let guid = f.guid();
        o.put_bytes(r + 12, &guid);
        put_wide_fixed(&mut o, r + 28, &f.friendly_name(), 257);
        put_wide_fixed(&mut o, r + 542, &f.description(), 257);
        if f.has_physical_address() {
            o.put_u32(r + 1056, 6);
            o.put_bytes(r + 1060, &f.mac);
            o.put_bytes(r + 1092, &f.mac);
        }
        o.put_u32(r + 1124, f.mtu);
        o.put_u32(r + 1128, f.if_type());
        o.put_u32(r + 1132, 0);
        let (medium, physical, access, flags) = match f.kind {
            Kind::Loopback => (MEDIUM_LOOPBACK, PHYSICAL_UNSPECIFIED, ACCESS_LOOPBACK, 0u8),
            Kind::Ethernet => (MEDIUM_802_3, PHYSICAL_802_3, ACCESS_BROADCAST, HARDWARE_INTERFACE | CONNECTOR_PRESENT),
        };
        o.put_u32(r + 1136, medium);
        o.put_u32(r + 1140, physical);
        o.put_u32(r + 1144, access);
        o.put_u32(r + 1148, 0);
        o.put_u8(r + 1152, flags | if f.up { 0 } else { NOT_MEDIA_CONNECTED });
        o.put_u32(r + 1156, if f.up { OPER_UP } else { OPER_DOWN });
        o.put_u32(r + 1160, ADMIN_UP);
        o.put_u32(r + 1164, if f.up { MEDIA_CONNECTED } else { MEDIA_DISCONNECTED });
        o.put_u32(r + 1184, 1);
        o.put_u64(r + 1192, f.speed);
        o.put_u64(r + 1200, f.speed);
    }
    o.buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;

    #[test]
    fn rows_are_1352_bytes_and_carry_the_interface() {
        let ifs = [IfInfo::loopback(), eth()];
        let b = table(&ifs);
        assert_eq!(b.len(), TABLE_HEADER + 2 * ROW_SIZE);
        assert_eq!(u32_at(&b, 0), 2);
        let r1 = TABLE_HEADER + ROW_SIZE;
        assert_eq!(u64_at(&b, r1), eth().luid());
        assert_eq!(u32_at(&b, r1 + 8), 2);
        assert_eq!(&b[r1 + 12..r1 + 28], &eth().guid());
        assert_eq!(wide_at(&b, r1 + 28), "Ethernet");
        assert_eq!(wide_at(&b, r1 + 542), "Poly VirtIO Ethernet Adapter");
        assert_eq!(u32_at(&b, r1 + 1056), 6);
        assert_eq!(&b[r1 + 1060..r1 + 1066], &eth().mac);
        assert_eq!(u32_at(&b, r1 + 1124), 1500);
        assert_eq!(u32_at(&b, r1 + 1128), 6);
        assert_eq!(u32_at(&b, r1 + 1136), 0);
        assert_eq!(u32_at(&b, r1 + 1144), 2);
        assert_eq!(b[r1 + 1152], HARDWARE_INTERFACE | CONNECTOR_PRESENT);
        assert_eq!(u32_at(&b, r1 + 1156), 1);
        assert_eq!(u32_at(&b, r1 + 1164), 1);
        assert_eq!(u64_at(&b, r1 + 1192), 10_000_000_000);
        // The loopback.
        let r0 = TABLE_HEADER;
        assert_eq!(u32_at(&b, r0 + 1128), 24);
        assert_eq!(u32_at(&b, r0 + 1136), 17);
        assert_eq!(u32_at(&b, r0 + 1144), 1);
        assert_eq!(u32_at(&b, r0 + 1056), 0);
    }

    #[test]
    fn a_down_interface_is_not_connected() {
        let mut e = eth();
        e.up = false;
        let b = table(&[e]);
        assert_eq!(u32_at(&b, TABLE_HEADER + 1156), 2);
        assert_eq!(u32_at(&b, TABLE_HEADER + 1164), 2);
        assert_eq!(b[TABLE_HEADER + 1152] & NOT_MEDIA_CONNECTED, NOT_MEDIA_CONNECTED);
    }

    #[test]
    fn an_empty_table_is_just_its_header() {
        let b = table(&[]);
        assert_eq!(b.len(), TABLE_HEADER);
        assert_eq!(u32_at(&b, 0), 0);
    }
}
