//! cachestat(2): the page cache state of a range of a file (ADR 0122).
//!
//! Structures from the UAPI header `linux/mman.h`: `struct
//! cachestat_range { __u64 off; __u64 len; }` and `struct cachestat {
//! __u64 nr_cache, nr_dirty, nr_writeback, nr_evicted, nr_recently_evicted;
//! }`. Poly's memory file systems (tmpfs, memfd) keep every page with data
//! in memory and never write it back or evict it, so only `nr_cache` is
//! ever non-zero: the pages of the range that hold data (holes of sparse
//! files count nothing). Descriptors without pages of their own (pipes,
//! sockets, directories, devices) report zeros.

use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::table::{Handler, nr};

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::CACHESTAT, cachestat)];

const PAGE: u64 = crate::abi::PAGE_SIZE;

/// Size of `struct cachestat_range`.
pub const RANGE_SIZE: usize = 16;
/// Size of `struct cachestat`.
pub const STAT_SIZE: usize = 40;

/// The pages `[first, end)` that overlap the byte range `off`, `len` (`len`
/// 0: to the end of the file, `end` is then `u64::MAX`).
pub fn pages(off: u64, len: u64) -> (u64, u64) {
    let first = off / PAGE;
    if len == 0 {
        return (first, u64::MAX);
    }
    let end = off.saturating_add(len);
    (first, end.div_ceil(PAGE).max(first + 1))
}

/// `struct cachestat` with `nr_cache` pages cached and nothing else.
pub fn encode(nr_cache: u64) -> [u8; STAT_SIZE] {
    let mut out = [0u8; STAT_SIZE];
    out[..8].copy_from_slice(&nr_cache.to_le_bytes());
    out
}

/// cachestat(fd, cstat_range, cstat, flags): `EINVAL` for flags other
/// than 0, `EBADF` for a descriptor that is not open (or `O_PATH`, the
/// kernel), `EFAULT` for an unreadable range or an unwritable result.
pub fn cachestat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.uint(3) != 0 {
        return Err(Errno::EINVAL);
    }
    let fd = a.fd(0);
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    let mut range = [0u8; RANGE_SIZE];
    k.copy_from_user(a.ptr(1), &mut range)?;
    let off = u64::from_le_bytes(range[..8].try_into().expect("8 bytes"));
    let len = u64::from_le_bytes(range[8..].try_into().expect("8 bytes"));
    let (first, end) = pages(off, len);
    let cached = k.cachestat(fd, first, end)?;
    k.copy_to_user(a.ptr(2), &encode(cached))?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_count_the_pages_they_touch() {
        assert_eq!(pages(0, 0), (0, u64::MAX));
        assert_eq!(pages(4096, 4096), (1, 2));
        assert_eq!(pages(1, 1), (0, 1));
        assert_eq!(pages(4000, 200), (0, 2));
        assert_eq!(pages(100_000, 0), (24, u64::MAX));
        assert_eq!(pages(u64::MAX - 10, 100), (u64::MAX / PAGE, u64::MAX / PAGE + 1));
        assert_eq!(&encode(3)[..8], &3u64.to_le_bytes());
        assert!(encode(3)[8..].iter().all(|&b| b == 0));
    }
}
