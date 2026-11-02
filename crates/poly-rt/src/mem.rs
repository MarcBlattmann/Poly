//! Memory mappings beyond the heap (frame buffers, large scratch space).

use crate::abi::prot;
use crate::{Result, sys};

pub use prot::{EXEC, READ, WRITE};

/// Map `len` bytes (rounded up to whole pages) of zeroed memory with
/// `prot` (`READ | WRITE`, ...) anywhere; returns its start.
pub fn map(len: usize, prot: u32) -> Result<*mut u8> {
    sys::map(0, len as u64, prot).map(|a| a as *mut u8)
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
