//! Safe structured exception handlers of a 32-bit image (ADR 0186, stage 4).
//!
//! A 32-bit image linked with `/SAFESEH` ("Image has Safe Exception Handlers", Learn: linker
//! options) carries a table of the handlers its frames may register: the load configuration
//! directory's `SEHandlerTable` (the VA of a sorted array of RVAs) and `SEHandlerCount` (PE
//! format, "The Load Configuration Structure"). The operating system calls no other handler of
//! that image. `IMAGE_DLLCHARACTERISTICS_NO_SEH` ("Does not use structured exception (SE)
//! handling. No SE handler may be called in this image") forbids all of them.
//!
//! [`handler_is_safe`] answers for one handler address from a **mapped** image (the section
//! layout of a loaded module, where an RVA is an offset): the loader's relocations have already
//! turned the table's VA into the module's actual one.

use crate::read::{u16_at, u32_at};

/// `IMAGE_DLLCHARACTERISTICS_NO_SEH`.
pub const DLL_NO_SEH: u16 = 0x0400;

/// The load configuration directory's index in the data directories.
const LOAD_CONFIG: usize = 10;
/// `SEHandlerTable` and `SEHandlerCount` in `IMAGE_LOAD_CONFIG_DIRECTORY32`, and the structure's
/// smallest `Size` that has them.
const SE_HANDLER_TABLE: usize = 64;
const SE_HANDLER_COUNT: usize = 68;
const SE_FIELDS_END: u32 = 72;

/// Whether `handler` (an address in the mapped 32-bit `image` loaded at `base`) may be called as
/// an exception handler of that image: its RVA is in the image's table; an image without a table
/// (linked without `/SAFESEH`, or older than the load configuration's SEH fields) allows any of
/// its code; one flagged `NO_SEH` allows none. An address outside the image, or an image that is
/// not PE32 or whose headers do not parse, is not judged here: `true`.
pub fn handler_is_safe(image: &[u8], base: u32, handler: u32) -> bool {
    let Some(rva) = handler.checked_sub(base) else { return true };
    if rva as usize >= image.len() {
        return true;
    }
    let Some(nt) = u32_at(image, 0x3C).map(|o| o as usize) else { return true };
    if u32_at(image, nt) != Some(0x0000_4550) {
        return true;
    }
    // PE32: the optional header follows the 20-byte file header; its magic is 0x10B.
    let opt = nt + 24;
    if u16_at(image, opt) != Some(0x10B) {
        return true;
    }
    if u16_at(image, opt + 70).unwrap_or(0) & DLL_NO_SEH != 0 {
        return false;
    }
    let directories = u32_at(image, opt + 92).unwrap_or(0) as usize;
    if directories <= LOAD_CONFIG {
        return true;
    }
    let Some(cfg) = u32_at(image, opt + 96 + 8 * LOAD_CONFIG).map(|r| r as usize) else { return true };
    if cfg == 0 || u32_at(image, cfg).unwrap_or(0) < SE_FIELDS_END {
        return true;
    }
    let table_va = u32_at(image, cfg + SE_HANDLER_TABLE).unwrap_or(0);
    let count = u32_at(image, cfg + SE_HANDLER_COUNT).unwrap_or(0) as usize;
    if table_va == 0 {
        return true;
    }
    let Some(table) = table_va.checked_sub(base).map(|t| t as usize) else { return true };
    if table.checked_add(count.saturating_mul(4)).is_none_or(|end| end > image.len()) {
        return true;
    }
    // The table is sorted: binary search.
    let (mut low, mut high) = (0usize, count);
    while low < high {
        let mid = low + (high - low) / 2;
        let entry = u32_at(image, table + 4 * mid).unwrap_or(0);
        match entry.cmp(&rva) {
            core::cmp::Ordering::Equal => return true,
            core::cmp::Ordering::Less => low = mid + 1,
            core::cmp::Ordering::Greater => high = mid,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::vec::Vec;

    const BASE: u32 = 0x0040_0000;

    /// A mapped PE32 image of 0x3000 bytes whose load configuration (at 0x2000, `size` bytes of
    /// it) names a handler table of `handlers` RVAs at 0x2800 (none: no table), with the given
    /// DLL characteristics and `directories` data directories.
    fn image(characteristics: u16, directories: u32, size: u32, handlers: Option<&[u32]>) -> Vec<u8> {
        let mut b = std::vec![0u8; 0x3000];
        let put32 = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        b[0] = 0x4D;
        b[1] = 0x5A;
        put32(&mut b, 0x3C, 0x80);
        put32(&mut b, 0x80, 0x0000_4550);
        // The file header's machine is not looked at; the optional header (PE32) at 0x98.
        b[0x98..0x9A].copy_from_slice(&0x10Bu16.to_le_bytes());
        b[0x98 + 70..0x98 + 72].copy_from_slice(&characteristics.to_le_bytes());
        put32(&mut b, 0x98 + 92, directories);
        put32(&mut b, 0x98 + 96 + 80, 0x2000);
        put32(&mut b, 0x98 + 96 + 84, size);
        put32(&mut b, 0x2000, size);
        if let Some(h) = handlers {
            put32(&mut b, 0x2000 + SE_HANDLER_TABLE, BASE + 0x2800);
            put32(&mut b, 0x2000 + SE_HANDLER_COUNT, h.len() as u32);
            for (i, rva) in h.iter().enumerate() {
                put32(&mut b, 0x2800 + 4 * i, *rva);
            }
        }
        b
    }

    #[test]
    fn only_the_handlers_in_the_table_are_safe() {
        let img = image(0, 16, 0x48, Some(&[0x1010, 0x1100, 0x1234, 0x1ff0]));
        for ok in [0x1010, 0x1100, 0x1234, 0x1ff0] {
            assert!(handler_is_safe(&img, BASE, BASE + ok), "{ok:#x}");
        }
        for bad in [0x1000, 0x1011, 0x1233, 0x1fff] {
            assert!(!handler_is_safe(&img, BASE, BASE + bad), "{bad:#x}");
        }
    }

    #[test]
    fn an_image_with_an_empty_table_allows_no_handler() {
        let img = image(0, 16, 0x48, Some(&[]));
        assert!(!handler_is_safe(&img, BASE, BASE + 0x1010));
    }

    #[test]
    fn an_image_without_a_table_allows_any_of_its_code() {
        // No SEH fields in the load configuration (a short structure), no table, and too few
        // data directories.
        assert!(handler_is_safe(&image(0, 16, 0x40, None), BASE, BASE + 0x1010));
        assert!(handler_is_safe(&image(0, 16, 0x48, None), BASE, BASE + 0x1010));
        assert!(handler_is_safe(&image(0, 10, 0x48, Some(&[0x1])), BASE, BASE + 0x1010));
    }

    #[test]
    fn no_seh_images_allow_none() {
        let img = image(DLL_NO_SEH, 16, 0x48, None);
        assert!(!handler_is_safe(&img, BASE, BASE + 0x1010));
        // Also with a table that lists it.
        assert!(!handler_is_safe(&image(DLL_NO_SEH, 16, 0x48, Some(&[0x1010])), BASE, BASE + 0x1010));
    }

    #[test]
    fn what_is_not_in_the_image_is_not_judged() {
        let img = image(0, 16, 0x48, Some(&[0x1010]));
        assert!(handler_is_safe(&img, BASE, 0x0012_0000)); // below the image (a stack or heap)
        assert!(handler_is_safe(&img, BASE, BASE + 0x9000)); // past it
        assert!(handler_is_safe(&[0u8; 16], BASE, BASE + 4)); // not a PE
    }
}
