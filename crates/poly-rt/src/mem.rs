//! Memory mappings beyond the heap: anonymous memory, mappings of handles
//! (a client's shared memory, a DRM buffer, a frame buffer), and memory
//! objects (anonymous files that can be sealed and passed on).

use crate::abi::prot;
use crate::fs::File;
use crate::{Result, sys};

pub use crate::abi::seal;
pub use prot::{EXEC, READ, WRITE};

/// Map `len` bytes (rounded up to whole pages) of zeroed memory with
/// `prot` (`READ | WRITE`, ...) anywhere; returns its start.
pub fn map(len: usize, prot: u32) -> Result<*mut u8> {
    sys::map(0, len as u64, prot).map(|a| a as *mut u8)
}

/// Map `len` bytes of the file `handle` is open on, from `offset` (a
/// multiple of the page size), shared with it and every other mapping of
/// it (`shared`) or as a private copy.
pub fn map_handle(handle: u32, offset: u64, len: usize, prot: u32, shared: bool) -> Result<*mut u8> {
    sys::map_handle(handle, offset, len as u64, prot, shared).map(|a| a as *mut u8)
}

/// Unmap `[ptr, ptr + len)`.
///
/// # Safety
///
/// Nothing may use the memory afterwards; it must not be the heap's.
pub unsafe fn unmap(ptr: *mut u8, len: usize) -> Result<()> {
    sys::unmap(ptr as u64, len as u64)
}

/// Change the protection of `[ptr, ptr + len)`.
///
/// # Safety
///
/// Code that uses the memory must not need the access taken away.
pub unsafe fn protect(ptr: *mut u8, len: usize, prot: u32) -> Result<()> {
    sys::protect(ptr as u64, len as u64, prot)
}

/// A memory object of `size` bytes: an anonymous file to fill, seal
/// ([`seal`]), map and pass on.
pub fn memory_object(size: u64) -> Result<File> {
    sys::memory_object(size).map(File::from_handle)
}

/// Add `seals` (`seal::*`) to a memory object.
pub fn seal_object(file: &File, seals: u32) -> Result<()> {
    sys::seal(file.handle(), seals)
}
