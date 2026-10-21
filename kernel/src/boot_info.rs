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
    /// The initial RAM disk the boot loader put in memory (QEMU's
    /// `-initrd`): the first boot module, if there is one.
    pub initrd: Option<Module>,
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

    /// A copy whose command line keeps only the words `keep` accepts (the
    /// shell runs the kernel tests without the boot's `poly.selftest.*`
    /// requests to fail or panic).
    pub fn filtered(&self, keep: impl Fn(&str) -> bool) -> BootInfo {
        let mut copy = BootInfo {
            memory: self.memory.clone(),
            rsdp: self.rsdp,
            initrd: self.initrd,
            cmdline: [0; 512],
            cmdline_len: 0,
        };
        for word in self.cmdline().split_ascii_whitespace().filter(|w| keep(w)) {
            let start = copy.cmdline_len + usize::from(copy.cmdline_len > 0);
            // Every kept word fits: it fitted in the original line.
            copy.cmdline[start..start + word.len()].copy_from_slice(word.as_bytes());
            if start > 0 {
                copy.cmdline[start - 1] = b' ';
            }
            copy.cmdline_len = start + word.len();
        }
        copy
    }

    /// The value of the first `key=value` word on the command line.
    pub fn arg(&self, key: &str) -> Option<&str> {
        self.cmdline().split_ascii_whitespace().find_map(|w| w.strip_prefix(key)?.strip_prefix('='))
    }

    /// The values of every `key=value` word on the command line, in order.
    pub fn args<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.cmdline().split_ascii_whitespace().filter_map(move |w| w.strip_prefix(key)?.strip_prefix('='))
    }
}

/// A boot module: bytes the boot loader loaded into physical memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Module {
    pub phys: u64,
    pub len: u64,
}

impl Module {
    /// Where it ends: it lies at `[phys, end())`.
    pub const fn end(&self) -> u64 {
        self.phys + self.len
    }
}

#[derive(Debug)]
#[allow(dead_code, reason = "fields are shown through Debug in the boot panic")]
pub enum Error {
    BadMagic(u32),
    NoMemoryMap,
    Unmapped(u64),
    MemoryMapFull,
    /// A module whose range wraps around the address space.
    BadModule {
        phys: u64,
        len: u64,
    },
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

/// `hvm_modlist_entry` from the same header: one module, such as the
/// initial RAM disk QEMU passes for `-initrd`. Present in every version.
#[repr(C)]
struct HvmModlistEntry {
    paddr: u64,
    size: u64,
    cmdline_paddr: u64,
    reserved: u64,
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

        // `nr_modules` entries at `modlist_paddr`. The first is the initial
        // RAM disk; Poly has no use for others yet. An empty one is none.
        let mut initrd = None;
        if si.nr_modules > 0 && si.modlist_paddr != 0 {
            let m = &*ptr::<HvmModlistEntry>(si.modlist_paddr)?;
            if m.paddr.checked_add(m.size).is_none() {
                return Err(Error::BadModule { phys: m.paddr, len: m.size });
            }
            initrd = (m.size > 0).then_some(Module { phys: m.paddr, len: m.size });
        }

        let mut info = BootInfo {
            memory,
            rsdp: (si.rsdp_paddr != 0).then_some(si.rsdp_paddr),
            initrd,
            cmdline: [0; 512],
            cmdline_len: 0,
        };
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
