//! Page protections: checking a `PAGE_*` value, what the hardware maps for
//! it, and which view protections a section allows.
//!
//! Rules from Learn, "Memory Protection Constants":
//!
//! - exactly one of the eight base protections (`PAGE_NOACCESS` ...
//!   `PAGE_EXECUTE_WRITECOPY`), plus at most the modifiers `PAGE_GUARD`,
//!   `PAGE_NOCACHE`, `PAGE_WRITECOMBINE`;
//! - `PAGE_GUARD` "cannot be used with `PAGE_NOACCESS`";
//! - `PAGE_NOCACHE` "cannot be used with the `PAGE_GUARD`, `PAGE_NOACCESS`,
//!   or `PAGE_WRITECOMBINE` flags", and `PAGE_WRITECOMBINE` likewise;
//! - `PAGE_WRITECOPY` and `PAGE_EXECUTE_WRITECOPY` are "not supported by the
//!   VirtualAlloc or VirtualAllocEx functions": private memory refuses them.
//!
//! Anything else is `STATUS_INVALID_PAGE_PROTECTION` (to be confirmed by
//! the spec agent against real Windows for each case). Caching attributes
//! are recorded and reported but change nothing: all memory is write-back.

use crate::consts::*;
use poly_ntobj::NtStatus;
use poly_ntobj::status::STATUS_INVALID_PAGE_PROTECTION;

/// What the page tables give a page: the kernel maps it with these rights
/// (`execute` without `read` is mapped readable: x86 cannot do execute-only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct HwProt {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

impl HwProt {
    pub const NONE: HwProt = HwProt { read: false, write: false, execute: false };
    pub const R: HwProt = HwProt { read: true, write: false, execute: false };
    pub const RW: HwProt = HwProt { read: true, write: true, execute: false };
    pub const RX: HwProt = HwProt { read: true, write: false, execute: true };
    pub const RWX: HwProt = HwProt { read: true, write: true, execute: true };
}

const BASE_MASK: u32 = 0xFF;
const MODIFIERS: u32 = PAGE_GUARD | PAGE_NOCACHE | PAGE_WRITECOMBINE;

/// A checked `PAGE_*` value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Protection(u32);

impl Protection {
    /// Check `value` (see the module docs). `private`: for private memory,
    /// where the copy-on-write protections are refused.
    pub fn new(value: u32, private: bool) -> Result<Protection, NtStatus> {
        let base = value & BASE_MASK;
        let modifiers = value & !BASE_MASK;
        let ok = base.is_power_of_two()
            && modifiers & !MODIFIERS == 0
            && !(base == PAGE_NOACCESS && modifiers != 0)
            && !(modifiers & PAGE_NOCACHE != 0 && modifiers & (PAGE_GUARD | PAGE_WRITECOMBINE) != 0)
            && !(modifiers & PAGE_WRITECOMBINE != 0 && modifiers & PAGE_GUARD != 0)
            && !(private && (base == PAGE_WRITECOPY || base == PAGE_EXECUTE_WRITECOPY));
        if ok { Ok(Protection(value)) } else { Err(STATUS_INVALID_PAGE_PROTECTION) }
    }

    /// The `PAGE_*` value.
    pub const fn value(self) -> u32 {
        self.0
    }

    /// The base protection, without modifiers.
    pub const fn base(self) -> u32 {
        self.0 & BASE_MASK
    }

    pub const fn is_guard(self) -> bool {
        self.0 & PAGE_GUARD != 0
    }

    /// The same protection without `PAGE_GUARD` (after a guard page fired).
    pub const fn without_guard(self) -> Protection {
        Protection(self.0 & !PAGE_GUARD)
    }

    /// What the page tables map. A guard page is not accessible (its first
    /// access must fault); copy-on-write protections map like their
    /// writable counterparts in a private mapping (the kernel copies on
    /// the first write) and read-only in a shared one.
    pub fn hw(self, shared_mapping: bool) -> HwProt {
        if self.is_guard() {
            return HwProt::NONE;
        }
        match self.base() {
            PAGE_READONLY => HwProt::R,
            PAGE_READWRITE => HwProt::RW,
            PAGE_WRITECOPY if shared_mapping => HwProt::R,
            PAGE_WRITECOPY => HwProt::RW,
            PAGE_EXECUTE | PAGE_EXECUTE_READ => HwProt::RX,
            PAGE_EXECUTE_READWRITE => HwProt::RWX,
            PAGE_EXECUTE_WRITECOPY if shared_mapping => HwProt::RX,
            PAGE_EXECUTE_WRITECOPY => HwProt::RWX,
            _ => HwProt::NONE,
        }
    }

    pub const fn allows_execute(self) -> bool {
        matches!(self.base(), PAGE_EXECUTE | PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY)
    }

    pub const fn allows_shared_write(self) -> bool {
        matches!(self.base(), PAGE_READWRITE | PAGE_EXECUTE_READWRITE)
    }

    pub const fn is_copy_on_write(self) -> bool {
        matches!(self.base(), PAGE_WRITECOPY | PAGE_EXECUTE_WRITECOPY)
    }

    pub const fn allows_read(self) -> bool {
        self.base() != PAGE_NOACCESS && self.base() != PAGE_EXECUTE
    }
}

/// Whether a view (or a page of one) may get protection `view` in a section
/// created with page protection `section`, after the table of Learn
/// "MapViewOfFile" (`FILE_MAP_WRITE` needs a `PAGE_READWRITE` section,
/// `FILE_MAP_READ` and `FILE_MAP_COPY` any readable one, `FILE_MAP_EXECUTE`
/// an executable one). `PAGE_NOACCESS` is always allowed. Otherwise
/// `STATUS_SECTION_PROTECTION` (to be confirmed by the spec agent).
pub fn view_allowed(section: Protection, view: Protection) -> bool {
    let s = section.base();
    let readable = s != PAGE_NOACCESS && s != PAGE_EXECUTE;
    let writable = matches!(s, PAGE_READWRITE | PAGE_EXECUTE_READWRITE);
    let executable = matches!(s, PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY | PAGE_EXECUTE);
    match view.base() {
        PAGE_NOACCESS => true,
        PAGE_READONLY | PAGE_WRITECOPY => readable,
        PAGE_READWRITE => writable,
        PAGE_EXECUTE => executable,
        PAGE_EXECUTE_READ | PAGE_EXECUTE_WRITECOPY => executable && readable,
        PAGE_EXECUTE_READWRITE => executable && writable,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_and_invalid_values() {
        for base in [PAGE_NOACCESS, PAGE_READONLY, PAGE_READWRITE, PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE]
        {
            assert!(Protection::new(base, true).is_ok(), "{base:#x}");
        }
        assert!(Protection::new(PAGE_WRITECOPY, false).is_ok());
        assert_eq!(Protection::new(PAGE_WRITECOPY, true), Err(STATUS_INVALID_PAGE_PROTECTION));
        assert_eq!(Protection::new(PAGE_EXECUTE_WRITECOPY, true), Err(STATUS_INVALID_PAGE_PROTECTION));
        assert_eq!(Protection::new(0, true), Err(STATUS_INVALID_PAGE_PROTECTION));
        assert_eq!(Protection::new(PAGE_READONLY | PAGE_READWRITE, true), Err(STATUS_INVALID_PAGE_PROTECTION));
        assert!(Protection::new(PAGE_READWRITE | PAGE_GUARD, true).is_ok());
        assert!(Protection::new(PAGE_READWRITE | PAGE_NOCACHE, true).is_ok());
        assert!(Protection::new(PAGE_READWRITE | PAGE_WRITECOMBINE, true).is_ok());
        assert_eq!(Protection::new(PAGE_NOACCESS | PAGE_GUARD, true), Err(STATUS_INVALID_PAGE_PROTECTION));
        assert_eq!(Protection::new(PAGE_READWRITE | PAGE_GUARD | PAGE_NOCACHE, true), Err(STATUS_INVALID_PAGE_PROTECTION));
        assert_eq!(
            Protection::new(PAGE_READWRITE | PAGE_NOCACHE | PAGE_WRITECOMBINE, true),
            Err(STATUS_INVALID_PAGE_PROTECTION)
        );
        assert_eq!(Protection::new(PAGE_READWRITE | 0x800, true), Err(STATUS_INVALID_PAGE_PROTECTION));
    }

    #[test]
    fn hardware_rights() {
        let p = |v| Protection::new(v, false).unwrap();
        assert_eq!(p(PAGE_NOACCESS).hw(false), HwProt::NONE);
        assert_eq!(p(PAGE_READWRITE | PAGE_GUARD).hw(false), HwProt::NONE);
        assert_eq!(p(PAGE_READWRITE | PAGE_GUARD).without_guard().hw(false), HwProt::RW);
        assert_eq!(p(PAGE_EXECUTE).hw(false), HwProt::RX);
        assert_eq!(p(PAGE_WRITECOPY).hw(false), HwProt::RW);
        assert_eq!(p(PAGE_WRITECOPY).hw(true), HwProt::R);
    }

    #[test]
    fn view_protections_follow_the_section() {
        let p = |v| Protection::new(v, false).unwrap();
        assert!(view_allowed(p(PAGE_READWRITE), p(PAGE_READWRITE)));
        assert!(view_allowed(p(PAGE_READWRITE), p(PAGE_READONLY)));
        assert!(view_allowed(p(PAGE_READWRITE), p(PAGE_WRITECOPY)));
        assert!(!view_allowed(p(PAGE_READONLY), p(PAGE_READWRITE)));
        assert!(view_allowed(p(PAGE_READONLY), p(PAGE_WRITECOPY)));
        assert!(!view_allowed(p(PAGE_READWRITE), p(PAGE_EXECUTE_READ)));
        assert!(view_allowed(p(PAGE_EXECUTE_READWRITE), p(PAGE_EXECUTE_READWRITE)));
        assert!(view_allowed(p(PAGE_READONLY), p(PAGE_NOACCESS)));
    }
}
