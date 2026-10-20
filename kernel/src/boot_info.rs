//! Boot information, independent of the boot protocol.
//!
//! Today the only protocol is PVH (QEMU `-kernel`). A UEFI loader for real
//! hardware will fill the same [`BootInfo`].

use crate::arch::phys_to_virt;
use poly_mem::{MemoryMap, Region, RegionKind};

#[derive(Debug)]
pub struct BootInfo {
    /// Raw firmware memory map (not yet normalized).
    pub memory: MemoryMap,
    pub rsdp: Option<u64>,
    cmdline: [u8; 512],
    cmdline_len: usize,
}

impl BootInfo {
    pub fn cmdline(&self) -> &str {
        core::str::from_utf8(&self.cmdline[..self.cmdline_len]).unwrap_or("")
    }

    /// True if the kernel command line contains `flag` as a whole word.
    pub fn has_flag(&self, flag: &str) -> bool {
        self.cmdline().split_ascii_whitespace().any(|w| w == flag)
    }

    /// The value of the first `key=value` word on the command line.
    pub fn arg(&self, key: &str) -> Option<&str> {
        self.cmdline().split_ascii_whitespace().find_map(|w| w.strip_prefix(key)?.strip_prefix('='))
    }
}

#[derive(Debug)]
#[allow(dead_code, reason = "fields are shown through Debug in the boot panic")]
pub enum Error {
    BadMagic(u32),
    NoMemoryMap,
    Unmapped(u64),
    MemoryMapFull,
}

/// `hvm_start_info` from Xen's public `arch-x86/hvm/start_info.h`.
#[repr(C)]
struct HvmStartInfo {
    magic: u32,
    version: u32,
    flags: u32,
    nr_modules: u32,
    modlist_paddr: u64,
    cmdline_paddr: u64,
    rsdp_paddr: u64,
    // version >= 1
    memmap_paddr: u64,
    memmap_entries: u32,
    reserved: u32,
}

#[repr(C)]
struct HvmMemmapEntry {
    addr: u64,
    size: u64,
    ty: u32,
    reserved: u32,
}

const HVM_START_MAGIC: u32 = 0x336e_c578;

fn ptr<T>(phys: u64) -> Result<*const T, Error> {
    phys_to_virt(phys).map(|v| v as *const T).ok_or(Error::Unmapped(phys))
}

/// Copy everything we need out of the PVH start info.
///
/// # Safety
/// `start_info` must be the physical address the PVH loader passed in `ebx`.
pub unsafe fn from_pvh(start_info: u64) -> Result<BootInfo, Error> {
    // SAFETY: the loader guarantees a valid hvm_start_info below 1 GiB; every
    // nested pointer is range-checked by `ptr` before it is read.
    unsafe {
        let si = &*ptr::<HvmStartInfo>(start_info)?;
        if si.magic != HVM_START_MAGIC {
            return Err(Error::BadMagic(si.magic));
        }
        if si.version < 1 || si.memmap_paddr == 0 {
            return Err(Error::NoMemoryMap);
        }

        let mut memory = MemoryMap::new();
        let entries = ptr::<HvmMemmapEntry>(si.memmap_paddr)?;
        for i in 0..si.memmap_entries as usize {
            let e = &*entries.add(i);
            let end = e.addr.saturating_add(e.size);
            memory.push(Region::new(e.addr, end, RegionKind::from_e820(e.ty))).map_err(|_| Error::MemoryMapFull)?;
        }

        let mut info =
            BootInfo { memory, rsdp: (si.rsdp_paddr != 0).then_some(si.rsdp_paddr), cmdline: [0; 512], cmdline_len: 0 };
        if si.cmdline_paddr != 0 {
            let src = ptr::<u8>(si.cmdline_paddr)?;
            while info.cmdline_len < info.cmdline.len() {
                let b = *src.add(info.cmdline_len);
                if b == 0 {
                    break;
                }
                info.cmdline[info.cmdline_len] = b;
                info.cmdline_len += 1;
            }
        }
        Ok(info)
    }
}
