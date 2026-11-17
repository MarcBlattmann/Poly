//! The system volume opened as a device (`\\.\C:`, `\??\C:`): what Google's installers open before
//! they unpack (`specs/windows/kernel32.dll/OpenVolume.yaml`).
//!
//! Poly's `C:` is a directory of its VFS, not a block device, so the volume handle is a stand-in
//! with the answers the sheet observed: a disk device (`FILE_DEVICE_DISK`, characteristics
//! 0x220020), one storage device and one disk extent, and for a handle with read access the first
//! sector of an NTFS volume and the NTFS volume data. The rights: attribute, control and
//! synchronize rights for everyone, the data of the volume for the local system and the
//! Administrators group ([`VOLUME_SDDL`]): the administrator's filtered token (Administrators
//! deny-only) is refused, the full token is not.
//!
//! Constants: `winioctl.h` (`IOCTL_STORAGE_GET_DEVICE_NUMBER`,
//! `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS`, `FSCTL_GET_NTFS_VOLUME_DATA`, `STORAGE_DEVICE_NUMBER`,
//! `VOLUME_DISK_EXTENTS`, `NTFS_VOLUME_DATA_BUFFER`), `ntstatus.h`.

use super::consts::*;
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::status::{STATUS_BUFFER_TOO_SMALL, STATUS_INVALID_PARAMETER};

/// The volume device's descriptor: everyone may read its attributes and descriptor, wait on it
/// and traverse it (the sheet: `FILE_TRAVERSE` opens for both tokens), the local system and the
/// Administrators group may do everything.
pub const VOLUME_SDDL: &str = "O:BAG:SYD:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;0x1200A0;;;WD)";

/// `$Volume` below the volume's root (NTFS's file for the volume): the local system and the
/// Administrators group only.
pub const VOLUME_FILE_SDDL: &str = "O:BAG:SYD:P(A;;FA;;;SY)(A;;FA;;;BA)";

/// The rights an open of the volume device asks without reaching its data: such an open answers
/// `Information` 0 (the sheet), any other right `FILE_OPENED`.
pub const LIGHT_RIGHTS: u32 = FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE;

/// The volume's sector and cluster sizes.
pub const BYTES_PER_SECTOR: u64 = 512;
pub const BYTES_PER_CLUSTER: u32 = 4096;

/// `IOCTL_STORAGE_GET_DEVICE_NUMBER`: `CTL_CODE(IOCTL_STORAGE_BASE 0x2D, 0x420, METHOD_BUFFERED,
/// FILE_ANY_ACCESS)`.
pub const IOCTL_STORAGE_GET_DEVICE_NUMBER: u32 = 0x002D_1080;
/// `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS`: `CTL_CODE(IOCTL_VOLUME_BASE 0x56, 0, METHOD_BUFFERED,
/// FILE_ANY_ACCESS)`.
pub const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x0056_0000;
/// `FSCTL_GET_NTFS_VOLUME_DATA`: `CTL_CODE(FILE_DEVICE_FILE_SYSTEM, 25, METHOD_BUFFERED,
/// FILE_ANY_ACCESS)`.
pub const FSCTL_GET_NTFS_VOLUME_DATA: u32 = 0x0009_0064;
/// `FILE_DEVICE_DISK`'s characteristics the sheet saw on the volume (`FileFsDeviceInformation`).
pub const VOLUME_CHARACTERISTICS: u32 = 0x0022_0020;
/// Where the volume starts on its disk (1 MiB, the usual first partition).
const PARTITION_OFFSET: u64 = 1 << 20;

/// What a volume handle knows of its volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeFacts {
    pub serial: u32,
    /// The volume's size in bytes.
    pub size: u64,
    /// Its free bytes.
    pub free: u64,
}

impl VolumeFacts {
    fn sectors(&self) -> u64 {
        self.size / BYTES_PER_SECTOR
    }

    fn clusters(&self) -> u64 {
        self.size / u64::from(BYTES_PER_CLUSTER)
    }
}

/// The first sector of the volume, an NTFS boot sector: the jump, the file system name `NTFS    `
/// at 3, the BIOS parameter block (bytes per sector, sectors per cluster, the total sectors at 40,
/// the serial at 72) and the 0x55 0xAA signature at 510.
pub fn boot_sector(v: &VolumeFacts) -> [u8; 512] {
    let mut b = [0u8; 512];
    b[0..3].copy_from_slice(&[0xEB, 0x52, 0x90]);
    b[3..11].copy_from_slice(b"NTFS    ");
    b[11..13].copy_from_slice(&(BYTES_PER_SECTOR as u16).to_le_bytes());
    b[13] = (u64::from(BYTES_PER_CLUSTER) / BYTES_PER_SECTOR) as u8;
    b[21] = 0xF8;
    b[24..26].copy_from_slice(&63u16.to_le_bytes());
    b[26..28].copy_from_slice(&255u16.to_le_bytes());
    b[28..32].copy_from_slice(&((PARTITION_OFFSET / BYTES_PER_SECTOR) as u32).to_le_bytes());
    b[40..48].copy_from_slice(&v.sectors().saturating_sub(1).to_le_bytes());
    b[72..80].copy_from_slice(&u64::from(v.serial).to_le_bytes());
    b[510] = 0x55;
    b[511] = 0xAA;
    b
}

/// A read of `len` bytes at `offset`: whole sectors only (`STATUS_INVALID_PARAMETER` otherwise,
/// the sheet's `ERROR_INVALID_PARAMETER` for 100 bytes); the boot sector, zeros after it, nothing
/// past the end.
pub fn read(v: &VolumeFacts, offset: u64, len: usize) -> Result<Vec<u8>, NtStatus> {
    if !offset.is_multiple_of(BYTES_PER_SECTOR) || !(len as u64).is_multiple_of(BYTES_PER_SECTOR) {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let end = offset.saturating_add(len as u64).min(v.size);
    let n = end.saturating_sub(offset) as usize;
    let mut out = alloc::vec![0u8; n];
    if offset < BYTES_PER_SECTOR && n > 0 {
        let sector = boot_sector(v);
        let k = (BYTES_PER_SECTOR - offset) as usize;
        out[..k.min(n)].copy_from_slice(&sector[offset as usize..offset as usize + k.min(n)]);
    }
    Ok(out)
}

/// A device control on the volume handle: the status and the output (cut to `outlen` by the
/// caller). `granted` is the handle's access: the NTFS volume data needs read access
/// (`STATUS_INVALID_DEVICE_REQUEST` without it, the sheet's `ERROR_INVALID_FUNCTION`).
pub fn control(v: &VolumeFacts, code: u32, granted: u32, outlen: usize) -> Result<Vec<u8>, NtStatus> {
    let need = |n: usize| if outlen < n { Err(STATUS_BUFFER_TOO_SMALL) } else { Ok(()) };
    match code {
        IOCTL_STORAGE_GET_DEVICE_NUMBER => {
            need(12)?;
            // DeviceType FILE_DEVICE_DISK, DeviceNumber 0, PartitionNumber 1.
            let mut b = Vec::with_capacity(12);
            for x in [FILE_DEVICE_DISK, 0, 1] {
                b.extend_from_slice(&x.to_le_bytes());
            }
            Ok(b)
        }
        IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS => {
            need(32)?;
            // NumberOfDiskExtents 1; DISK_EXTENT { DiskNumber 0, StartingOffset, ExtentLength }.
            let mut b = alloc::vec![0u8; 32];
            b[0..4].copy_from_slice(&1u32.to_le_bytes());
            b[16..24].copy_from_slice(&PARTITION_OFFSET.to_le_bytes());
            b[24..32].copy_from_slice(&v.size.to_le_bytes());
            Ok(b)
        }
        FSCTL_GET_NTFS_VOLUME_DATA => {
            if granted & FILE_READ_DATA == 0 {
                return Err(STATUS_INVALID_DEVICE_REQUEST);
            }
            need(96)?;
            let mut b = alloc::vec![0u8; 96];
            let q = |b: &mut [u8], at: usize, x: u64| b[at..at + 8].copy_from_slice(&x.to_le_bytes());
            q(&mut b, 0, u64::from(v.serial));
            q(&mut b, 8, v.sectors());
            q(&mut b, 16, v.clusters());
            q(&mut b, 24, v.free / u64::from(BYTES_PER_CLUSTER));
            b[40..44].copy_from_slice(&(BYTES_PER_SECTOR as u32).to_le_bytes());
            b[44..48].copy_from_slice(&BYTES_PER_CLUSTER.to_le_bytes());
            b[48..52].copy_from_slice(&1024u32.to_le_bytes());
            q(&mut b, 64, 4);
            q(&mut b, 72, 2);
            Ok(b)
        }
        _ => Err(STATUS_INVALID_DEVICE_REQUEST),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: VolumeFacts = VolumeFacts { serial: 0x5059_0001, size: 64 << 30, free: 32 << 30 };

    #[test]
    fn the_boot_sector() {
        let s = boot_sector(&V);
        assert_eq!(&s[3..11], b"NTFS    ");
        assert_eq!((s[510], s[511]), (0x55, 0xAA));
        let r = read(&V, 0, 512).unwrap();
        assert_eq!(r.as_slice(), &s[..]);
        assert_eq!(read(&V, 0, 1024).unwrap()[512..], [0u8; 512]);
        assert_eq!(read(&V, 512, 512).unwrap(), alloc::vec![0u8; 512]);
    }

    #[test]
    fn reads_are_whole_sectors() {
        assert_eq!(read(&V, 0, 100), Err(STATUS_INVALID_PARAMETER));
        assert_eq!(read(&V, 10, 512), Err(STATUS_INVALID_PARAMETER));
        assert_eq!(read(&V, V.size, 512).unwrap().len(), 0);
    }

    #[test]
    fn controls() {
        let n = control(&V, IOCTL_STORAGE_GET_DEVICE_NUMBER, 0, 12).unwrap();
        assert_eq!((n.len(), u32::from_le_bytes(n[0..4].try_into().unwrap())), (12, 7));
        let e = control(&V, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, 0, 64).unwrap();
        assert_eq!(u32::from_le_bytes(e[0..4].try_into().unwrap()), 1);
        assert_eq!(control(&V, FSCTL_GET_NTFS_VOLUME_DATA, 0, 96), Err(STATUS_INVALID_DEVICE_REQUEST));
        let d = control(&V, FSCTL_GET_NTFS_VOLUME_DATA, FILE_READ_DATA, 96).unwrap();
        assert_eq!(u32::from_le_bytes(d[44..48].try_into().unwrap()), 4096);
        assert_eq!(control(&V, 0x1234, FILE_READ_DATA, 96), Err(STATUS_INVALID_DEVICE_REQUEST));
        assert_eq!(control(&V, IOCTL_STORAGE_GET_DEVICE_NUMBER, 0, 4), Err(STATUS_BUFFER_TOO_SMALL));
    }
}
