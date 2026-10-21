//! Memory: the argument rules of `VirtualFree`, `VirtualProtect` and the
//! local/global heap functions (Learn's pages for each; `winnt.h` and
//! `winbase.h` constants).
//!
//! `LocalAlloc`/`GlobalAlloc` are served from the process heap. A
//! *moveable* block (`LMEM_MOVEABLE`, `GMEM_MOVEABLE`) is a fixed block in
//! Poly: its handle is its address, so `LocalLock` returns the handle and
//! `LocalHandle` the pointer (Poly's choice; Learn only promises that a
//! locked moveable block does not move, ADR 0078).

use crate::error::ERROR_INVALID_PARAMETER;

pub const MEM_COMMIT: u32 = 0x1000;
pub const MEM_RESERVE: u32 = 0x2000;
pub const MEM_DECOMMIT: u32 = 0x4000;
pub const MEM_RELEASE: u32 = 0x8000;
pub const MEM_RESET: u32 = 0x8_0000;
pub const MEM_TOP_DOWN: u32 = 0x10_0000;

/// `HEAP_*` flags (`winnt.h`).
pub const HEAP_NO_SERIALIZE: u32 = 0x1;
pub const HEAP_GROWABLE: u32 = 0x2;
pub const HEAP_GENERATE_EXCEPTIONS: u32 = 0x4;
pub const HEAP_ZERO_MEMORY: u32 = 0x8;
pub const HEAP_REALLOC_IN_PLACE_ONLY: u32 = 0x10;
pub const HEAP_CREATE_ENABLE_EXECUTE: u32 = 0x4_0000;

/// `LMEM_*` (`winbase.h`).
pub const LMEM_FIXED: u32 = 0x0;
pub const LMEM_MOVEABLE: u32 = 0x2;
pub const LMEM_NOCOMPACT: u32 = 0x10;
pub const LMEM_NODISCARD: u32 = 0x20;
pub const LMEM_ZEROINIT: u32 = 0x40;
pub const LMEM_MODIFY: u32 = 0x80;
pub const LMEM_DISCARDABLE: u32 = 0xF00;
pub const LMEM_VALID_FLAGS: u32 = 0xF72;
/// `GMEM_*` (`winbase.h`).
pub const GMEM_MOVEABLE: u32 = 0x2;
pub const GMEM_ZEROINIT: u32 = 0x40;
pub const GMEM_VALID_FLAGS: u32 = 0x7F72;

/// `VirtualFree`: Learn: with `MEM_RELEASE` the size must be 0; exactly one
/// of `MEM_DECOMMIT` and `MEM_RELEASE`.
pub fn check_free(size: usize, free_type: u32) -> Result<(), u32> {
    match free_type {
        MEM_RELEASE if size != 0 => Err(ERROR_INVALID_PARAMETER),
        MEM_RELEASE | MEM_DECOMMIT => Ok(()),
        _ => Err(ERROR_INVALID_PARAMETER),
    }
}

/// `HeapCreate`'s `RtlCreateHeap` flags: Learn: a maximum size of 0 makes
/// the heap growable; the options pass through (`HEAP_NO_SERIALIZE`,
/// `HEAP_GENERATE_EXCEPTIONS`, `HEAP_CREATE_ENABLE_EXECUTE`).
pub fn heap_create_flags(options: u32, maximum: usize) -> u32 {
    let keep = HEAP_NO_SERIALIZE | HEAP_GENERATE_EXCEPTIONS | HEAP_CREATE_ENABLE_EXECUTE;
    (options & keep) | if maximum == 0 { HEAP_GROWABLE } else { 0 }
}

/// `LocalAlloc`: the heap flags for `flags`, or `ERROR_INVALID_PARAMETER`
/// for a flag `LMEM_VALID_FLAGS` does not have.
pub fn local_alloc_flags(flags: u32) -> Result<u32, u32> {
    if flags & !LMEM_VALID_FLAGS != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    Ok(if flags & LMEM_ZEROINIT != 0 { HEAP_ZERO_MEMORY } else { 0 })
}

/// `GlobalAlloc`: as [`local_alloc_flags`] for `GMEM_VALID_FLAGS`
/// (`GMEM_DDESHARE`, `GMEM_SHARE` and the other legacy flags are
/// accepted and have no effect, as Learn says for 32/64-bit Windows).
pub fn global_alloc_flags(flags: u32) -> Result<u32, u32> {
    if flags & !GMEM_VALID_FLAGS != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    Ok(if flags & GMEM_ZEROINIT != 0 { HEAP_ZERO_MEMORY } else { 0 })
}

/// `LocalReAlloc`: the heap flags. Learn: without `LMEM_MOVEABLE` a fixed
/// block may only be resized in place; `LMEM_MODIFY` changes attributes
/// only (Poly: nothing to change, so the block stays as it is).
pub fn local_realloc_flags(flags: u32) -> u32 {
    let mut f = 0;
    if flags & LMEM_ZEROINIT != 0 {
        f |= HEAP_ZERO_MEMORY;
    }
    if flags & LMEM_MOVEABLE == 0 {
        f |= HEAP_REALLOC_IN_PLACE_ONLY;
    }
    f
}

/// `MEMORY_BASIC_INFORMATION` (`winnt.h`, x64): 48 bytes.
pub const MEMORY_BASIC_INFORMATION_SIZE: usize = 48;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_free_rules() {
        assert_eq!(check_free(0, MEM_RELEASE), Ok(()));
        assert_eq!(check_free(4096, MEM_RELEASE), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(check_free(4096, MEM_DECOMMIT), Ok(()));
        assert_eq!(check_free(0, MEM_DECOMMIT | MEM_RELEASE), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(check_free(0, 0), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn heap_flags() {
        assert_eq!(heap_create_flags(0, 0), HEAP_GROWABLE);
        assert_eq!(heap_create_flags(HEAP_NO_SERIALIZE, 1 << 20), HEAP_NO_SERIALIZE);
        assert_eq!(heap_create_flags(HEAP_ZERO_MEMORY, 0), HEAP_GROWABLE);
        assert_eq!(local_alloc_flags(LMEM_FIXED), Ok(0));
        assert_eq!(local_alloc_flags(LMEM_MOVEABLE | LMEM_ZEROINIT), Ok(HEAP_ZERO_MEMORY));
        assert_eq!(local_alloc_flags(0x1), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(global_alloc_flags(0x2000 | GMEM_ZEROINIT), Ok(HEAP_ZERO_MEMORY));
        assert_eq!(global_alloc_flags(0x8000), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(local_realloc_flags(LMEM_MOVEABLE), 0);
        assert_eq!(local_realloc_flags(LMEM_ZEROINIT), HEAP_ZERO_MEMORY | HEAP_REALLOC_IN_PLACE_ONLY);
    }
}
