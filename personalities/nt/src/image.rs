//! Image sections (`SEC_IMAGE`): a PE executable or DLL laid out as it is
//! mapped, with the protection of every page, using `poly-pe`'s load plan
//! (ADR 0029).
//!
//! - Only x64 PE32+ images are accepted; anything else is
//!   `STATUS_INVALID_IMAGE_FORMAT` (to be confirmed by the spec agent,
//!   which may find `STATUS_INVALID_IMAGE_WIN_32` or a machine mismatch
//!   status for some cases).
//! - Page protections follow the sections' `IMAGE_SCN_MEM_*` flags (PE
//!   Format, "Section Flags"): the headers are `PAGE_READONLY`; readable
//!   sections `PAGE_READONLY`, writable ones `PAGE_WRITECOPY` (views of an
//!   image are copy on write, Learn: "Memory Protection Constants"),
//!   executable ones `PAGE_EXECUTE_READ` / `PAGE_EXECUTE_WRITECOPY`, a
//!   section without any access `PAGE_NOACCESS`. A page shared by two
//!   sections (alignments below the page size) gets the union of their
//!   rights. Gaps between sections are `PAGE_NOACCESS`. The exact values
//!   Windows reports are to be confirmed by the spec agent.

use crate::consts::*;
use crate::protect::Protection;
use crate::vm::{PAGE_SIZE, page_up};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::status::STATUS_INVALID_IMAGE_FORMAT;
use poly_pe::{Bits, ImageInfo, Pe};

/// An image laid out in memory, ready to be mapped.
#[derive(Clone, Debug)]
pub struct ImageSection {
    /// The image's bytes at their RVAs, `size` long.
    pub image: Arc<[u8]>,
    pub info: ImageInfo,
    /// Bytes of address space: `SizeOfImage` rounded up to a page.
    pub size: u64,
    /// The protection of each page.
    pub pages: Vec<Protection>,
}

#[derive(Clone, Copy, Default)]
struct Rights {
    read: bool,
    write: bool,
    execute: bool,
}

impl Rights {
    fn protection(self) -> u32 {
        match (self.read || self.write, self.write, self.execute) {
            (_, true, true) => PAGE_EXECUTE_WRITECOPY,
            (true, false, true) => PAGE_EXECUTE_READ,
            (false, false, true) => PAGE_EXECUTE,
            (_, true, false) => PAGE_WRITECOPY,
            (true, false, false) => PAGE_READONLY,
            (false, false, false) => PAGE_NOACCESS,
        }
    }
}

impl ImageSection {
    /// Parse and lay out the image file `file`.
    pub fn from_file(file: &[u8]) -> Result<ImageSection, NtStatus> {
        let bad = |_| STATUS_INVALID_IMAGE_FORMAT;
        let pe = Pe::parse(file).map_err(bad)?;
        let info = *pe.info();
        if info.machine() != poly_pe::machine::AMD64 || info.bits() != Bits::Pe32Plus {
            return Err(STATUS_INVALID_IMAGE_FORMAT);
        }
        let plan = pe.load_plan().map_err(bad)?;
        let size = page_up(u64::from(plan.size_of_image)).ok_or(STATUS_INVALID_IMAGE_FORMAT)?;
        let mut image = vec![0u8; size as usize];
        plan.map_into(&mut image).map_err(bad)?;
        let mut rights = vec![Rights::default(); (size / PAGE_SIZE) as usize];
        for region in plan.regions() {
            let start = u64::from(region.rva) / PAGE_SIZE;
            let end = page_up(u64::from(region.rva) + u64::from(region.size)).unwrap_or(size).min(size) / PAGE_SIZE;
            for r in &mut rights[start as usize..end as usize] {
                r.read |= region.protection.read;
                r.write |= region.protection.write;
                r.execute |= region.protection.execute;
            }
        }
        let pages = rights.iter().map(|r| Protection::new(r.protection(), false).expect("a base protection")).collect();
        Ok(ImageSection { image: image.into(), info, size, pages })
    }

    pub fn image_base(&self) -> u64 {
        self.info.image_base()
    }

    pub fn relocatable(&self) -> bool {
        self.info.relocatable()
    }

    /// A copy of the image relocated to load at `base` (ADR 0029:
    /// `poly_pe::relocate` validates the whole table before writing).
    pub fn relocated(&self, base: u64) -> Result<Arc<[u8]>, NtStatus> {
        let mut copy = self.image.to_vec();
        poly_pe::relocate(&mut copy, &self.info, base).map_err(|_| STATUS_INVALID_IMAGE_FORMAT)?;
        Ok(copy.into())
    }
}
