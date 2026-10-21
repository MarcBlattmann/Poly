//! File mappings: `CreateFileMappingW`'s protection and attributes,
//! `MapViewOfFile(Ex)`'s access, as the section services take them.
//!
//! Learn: `CreateFileMappingW` ("flProtect": one page protection, combined
//! with `SEC_*` attributes; `SEC_COMMIT` when none is given; a mapping of
//! the paging file needs a size), `MapViewOfFile` / `MapViewOfFileEx`
//! ("dwDesiredAccess": `FILE_MAP_*`, optionally with `FILE_MAP_COPY` or
//! `FILE_MAP_EXECUTE`; "the offset must be a multiple of the allocation
//! granularity"). Constants: `winnt.h` (`PAGE_*`, `SEC_*`, `SECTION_*`),
//! `memoryapi.h` (`FILE_MAP_*`), `winerror.h`.

use crate::error::*;

/// `PAGE_*` (`winnt.h`).
pub const PAGE_NOACCESS: u32 = 0x01;
pub const PAGE_READONLY: u32 = 0x02;
pub const PAGE_READWRITE: u32 = 0x04;
pub const PAGE_WRITECOPY: u32 = 0x08;
pub const PAGE_EXECUTE: u32 = 0x10;
pub const PAGE_EXECUTE_READ: u32 = 0x20;
pub const PAGE_EXECUTE_READWRITE: u32 = 0x40;
pub const PAGE_EXECUTE_WRITECOPY: u32 = 0x80;

/// `SEC_*` (`winnt.h`).
pub const SEC_IMAGE: u32 = 0x0100_0000;
pub const SEC_RESERVE: u32 = 0x0400_0000;
pub const SEC_COMMIT: u32 = 0x0800_0000;
pub const SEC_NOCACHE: u32 = 0x1000_0000;
pub const SEC_WRITECOMBINE: u32 = 0x4000_0000;
pub const SEC_LARGE_PAGES: u32 = 0x8000_0000;
const SEC_MASK: u32 = SEC_IMAGE | SEC_RESERVE | SEC_COMMIT | SEC_NOCACHE | SEC_WRITECOMBINE | SEC_LARGE_PAGES;

/// `SECTION_*` access rights (`winnt.h`).
pub const SECTION_QUERY: u32 = 0x0001;
pub const SECTION_MAP_WRITE: u32 = 0x0002;
pub const SECTION_MAP_READ: u32 = 0x0004;
pub const SECTION_MAP_EXECUTE: u32 = 0x0008;
pub const SECTION_EXTEND_SIZE: u32 = 0x0010;
pub const SECTION_ALL_ACCESS: u32 = 0x000F_001F;
/// `STANDARD_RIGHTS_REQUIRED` (`winnt.h`).
pub const STANDARD_RIGHTS_REQUIRED: u32 = 0x000F_0000;

/// `FILE_MAP_*` (`memoryapi.h`).
pub const FILE_MAP_COPY: u32 = SECTION_QUERY;
pub const FILE_MAP_WRITE: u32 = SECTION_MAP_WRITE;
pub const FILE_MAP_READ: u32 = SECTION_MAP_READ;
pub const FILE_MAP_ALL_ACCESS: u32 = SECTION_ALL_ACCESS;
pub const FILE_MAP_EXECUTE: u32 = 0x0020;

/// `ERROR_MAPPED_ALIGNMENT` (`winerror.h`).
pub const ERROR_MAPPED_ALIGNMENT: u32 = 1132;

/// The allocation granularity (`SYSTEM_INFO.dwAllocationGranularity`).
pub const ALLOCATION_GRANULARITY: u64 = 0x1_0000;

/// What `NtCreateSection` gets for a `CreateFileMappingW` call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SectionArgs {
    /// `SectionPageProtection`.
    pub protect: u32,
    /// `AllocationAttributes`.
    pub attributes: u32,
    /// `DesiredAccess` for the handle.
    pub access: u32,
}

/// `CreateFileMappingW`'s `flProtect` split into the section's page
/// protection and attributes, and the access its handle gets: read and
/// query always, write for a read-write protection, execute for an
/// execute one. `paging_file`: no file (`INVALID_HANDLE_VALUE`), which
/// needs a size (`size == 0` is `ERROR_INVALID_PARAMETER`) and cannot be
/// `SEC_IMAGE`.
pub fn create_args(protect: u32, paging_file: bool, size: u64) -> Result<SectionArgs, u32> {
    let page = protect & !SEC_MASK;
    let mut attributes = protect & SEC_MASK;
    let access = match page {
        PAGE_READONLY | PAGE_WRITECOPY => SECTION_QUERY | SECTION_MAP_READ,
        PAGE_READWRITE => SECTION_QUERY | SECTION_MAP_READ | SECTION_MAP_WRITE,
        PAGE_EXECUTE_READ | PAGE_EXECUTE_WRITECOPY => SECTION_QUERY | SECTION_MAP_READ | SECTION_MAP_EXECUTE,
        PAGE_EXECUTE_READWRITE => SECTION_QUERY | SECTION_MAP_READ | SECTION_MAP_WRITE | SECTION_MAP_EXECUTE,
        _ => return Err(ERROR_INVALID_PARAMETER),
    };
    if attributes & (SEC_IMAGE | SEC_RESERVE | SEC_COMMIT) == 0 {
        attributes |= SEC_COMMIT;
    }
    if paging_file && (size == 0 || attributes & SEC_IMAGE != 0) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    Ok(SectionArgs { protect: page, attributes, access: access | STANDARD_RIGHTS_REQUIRED })
}

/// `MapViewOfFile`'s `dwDesiredAccess` as a view protection: copy-on-write
/// for `FILE_MAP_COPY`, read-write for `FILE_MAP_WRITE` (and
/// `FILE_MAP_ALL_ACCESS`), read-only for `FILE_MAP_READ`, each with
/// execute added by `FILE_MAP_EXECUTE`. Nothing readable is
/// `ERROR_INVALID_PARAMETER`.
pub fn view_protect(access: u32) -> Result<u32, u32> {
    let exec = access & FILE_MAP_EXECUTE != 0;
    let p = if access & FILE_MAP_COPY != 0 && access & FILE_MAP_WRITE == 0 {
        if exec { PAGE_EXECUTE_WRITECOPY } else { PAGE_WRITECOPY }
    } else if access & FILE_MAP_WRITE != 0 {
        if exec { PAGE_EXECUTE_READWRITE } else { PAGE_READWRITE }
    } else if access & FILE_MAP_READ != 0 {
        if exec { PAGE_EXECUTE_READ } else { PAGE_READONLY }
    } else {
        return Err(ERROR_INVALID_PARAMETER);
    };
    // FILE_MAP_ALL_ACCESS holds SECTION_QUERY (= FILE_MAP_COPY) too: a
    // write view, not a copy.
    if access & SECTION_ALL_ACCESS == SECTION_ALL_ACCESS {
        return Ok(if exec { PAGE_EXECUTE_READWRITE } else { PAGE_READWRITE });
    }
    Ok(p)
}

/// The offset of a view: a multiple of the allocation granularity
/// (`ERROR_MAPPED_ALIGNMENT`).
pub fn view_offset(high: u32, low: u32) -> Result<u64, u32> {
    let off = (u64::from(high) << 32) | u64::from(low);
    if off % ALLOCATION_GRANULARITY != 0 { Err(ERROR_MAPPED_ALIGNMENT) } else { Ok(off) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_arguments() {
        let a = create_args(PAGE_READWRITE, true, 0x1000).unwrap();
        assert_eq!(a.protect, PAGE_READWRITE);
        assert_eq!(a.attributes, SEC_COMMIT);
        assert_eq!(a.access & 0xFFFF, SECTION_QUERY | SECTION_MAP_READ | SECTION_MAP_WRITE);
        let a = create_args(PAGE_READONLY | SEC_RESERVE, false, 0).unwrap();
        assert_eq!((a.protect, a.attributes), (PAGE_READONLY, SEC_RESERVE));
        assert_eq!(a.access & SECTION_MAP_WRITE, 0);
        assert_eq!(create_args(PAGE_READWRITE, true, 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(create_args(PAGE_READONLY | SEC_IMAGE, true, 1), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(create_args(PAGE_NOACCESS, false, 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(create_args(PAGE_READWRITE | 0x100, false, 0), Err(ERROR_INVALID_PARAMETER));
        let x = create_args(PAGE_EXECUTE_READWRITE, false, 0).unwrap();
        assert_eq!(x.access & SECTION_MAP_EXECUTE, SECTION_MAP_EXECUTE);
    }

    #[test]
    fn view_access() {
        assert_eq!(view_protect(FILE_MAP_READ), Ok(PAGE_READONLY));
        assert_eq!(view_protect(FILE_MAP_WRITE), Ok(PAGE_READWRITE));
        assert_eq!(view_protect(FILE_MAP_WRITE | FILE_MAP_READ), Ok(PAGE_READWRITE));
        assert_eq!(view_protect(FILE_MAP_ALL_ACCESS), Ok(PAGE_READWRITE));
        assert_eq!(view_protect(FILE_MAP_COPY), Ok(PAGE_WRITECOPY));
        assert_eq!(view_protect(FILE_MAP_READ | FILE_MAP_EXECUTE), Ok(PAGE_EXECUTE_READ));
        assert_eq!(view_protect(0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(view_offset(0, 0x2_0000), Ok(0x2_0000));
        assert_eq!(view_offset(1, 0), Ok(1 << 32));
        assert_eq!(view_offset(0, 0x1000), Err(ERROR_MAPPED_ALIGNMENT));
    }
}
