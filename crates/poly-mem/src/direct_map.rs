//! Which physical ranges a kernel's direct map covers.
//!
//! A direct map maps physical memory linearly at a fixed virtual offset, so
//! the kernel can reach any frame without creating a mapping first. Mapping
//! the whole physical address space would waste page tables on holes (and
//! map device windows nobody asked for), so the direct map covers exactly the
//! regions of the memory map, rounded out to whole pages. [`DirectMap`]
//! records those spans; the kernel maps each one and answers "is this
//! physical range reachable?" from it.

use crate::{MemoryMap, PAGE_SIZE, Region, RegionKind, align_down, align_up};

/// Page-aligned, sorted, non-overlapping spans of physical memory, each with
/// the kind of the region it came from.
#[derive(Clone, Debug)]
pub struct DirectMap {
    spans: [Region; MemoryMap::CAPACITY],
    len: usize,
}

impl Default for DirectMap {
    fn default() -> Self {
        DirectMap::new()
    }
}

impl DirectMap {
    /// A direct map that covers nothing.
    pub const fn new() -> DirectMap {
        DirectMap { spans: [Region::new(0, 0, RegionKind::Usable); MemoryMap::CAPACITY], len: 0 }
    }

    /// The spans for the normalized map `map`, below `limit`.
    ///
    /// Every region except [`RegionKind::Bad`] is covered, rounded out to
    /// whole pages: firmware regions are not always page-aligned, and the
    /// hardware maps nothing smaller. Holes between regions stay uncovered.
    /// When rounding makes two regions share a page, the page goes to the
    /// more restrictive kind (the same rule [`MemoryMap::normalized`] uses
    /// for overlaps), so a page that is partly device memory is never treated
    /// as ordinary RAM. Adjacent spans of the same kind are merged.
    pub fn from_map(map: &MemoryMap, limit: u64) -> DirectMap {
        let limit = align_down(limit, PAGE_SIZE);
        let mut dm = DirectMap::new();
        for r in map.regions().iter().filter(|r| r.kind != RegionKind::Bad) {
            let mut start = align_down(r.start, PAGE_SIZE);
            let end = align_up(r.end, PAGE_SIZE).min(limit);
            if start >= end {
                continue;
            }
            // `map` is sorted and non-overlapping, so only the last span can
            // reach into this region, and by less than a page on each side.
            if let Some(last) = dm.last_mut()
                && last.end > start
            {
                if r.kind > last.kind {
                    last.end = start;
                    if last.is_empty() {
                        dm.len -= 1;
                    }
                } else {
                    start = last.end;
                }
            }
            if start >= end {
                continue;
            }
            match dm.last_mut() {
                Some(last) if last.end == start && last.kind == r.kind => last.end = end,
                _ => {
                    // At most one span per region, so this always fits.
                    dm.spans[dm.len] = Region::new(start, end, r.kind);
                    dm.len += 1;
                }
            }
        }
        dm
    }

    pub fn spans(&self) -> &[Region] {
        &self.spans[..self.len]
    }

    /// End of the highest span: everything the direct map covers lies below.
    pub fn end(&self) -> u64 {
        self.spans().last().map_or(0, |s| s.end)
    }

    /// The span containing `phys`.
    pub fn find(&self, phys: u64) -> Option<&Region> {
        let spans = self.spans();
        let i = spans.partition_point(|s| s.end <= phys);
        spans.get(i).filter(|s| s.start <= phys)
    }

    /// Whether every byte of `[phys, phys + len)` is covered, possibly by
    /// several adjacent spans. An empty range counts as its first byte.
    pub fn covers(&self, phys: u64, len: u64) -> bool {
        let Some(last) = phys.checked_add(len.max(1) - 1) else {
            return false;
        };
        let spans = self.spans();
        let mut i = spans.partition_point(|s| s.end <= phys);
        match spans.get(i) {
            Some(s) if s.start <= phys => {}
            _ => return false,
        }
        while spans[i].end <= last {
            match spans.get(i + 1) {
                Some(next) if next.start == spans[i].end => i += 1,
                _ => return false,
            }
        }
        true
    }

    fn last_mut(&mut self) -> Option<&mut Region> {
        self.len.checked_sub(1).map(|i| &mut self.spans[i])
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use RegionKind::*;
    use std::vec::Vec;

    fn map(rs: &[(u64, u64, RegionKind)]) -> MemoryMap {
        let mut m = MemoryMap::new();
        for &(s, e, k) in rs {
            m.push(Region::new(s, e, k)).unwrap();
        }
        m.normalized().unwrap()
    }

    fn spans(dm: &DirectMap) -> Vec<(u64, u64, RegionKind)> {
        dm.spans().iter().map(|s| (s.start, s.end, s.kind)).collect()
    }

    #[test]
    fn covers_regions_and_leaves_holes() {
        // Shaped like QEMU q35 with 256 MiB: low RAM, firmware, an ECAM
        // window, and a reserved range far above RAM.
        let m = map(&[
            (0, 0x10_0000, Reserved),
            (0x10_0000, 0x13_2000, Kernel),
            (0x13_2000, 0xffd_f000, Usable),
            (0xffd_f000, 0x1000_0000, Reserved),
            (0xb000_0000, 0xc000_0000, Reserved),
            (0xfd_0000_0000, 0x100_0000_0000, Reserved),
        ]);
        let dm = DirectMap::from_map(&m, u64::MAX);
        assert_eq!(
            spans(&dm),
            [
                (0, 0x10_0000, Reserved),
                (0x10_0000, 0x13_2000, Kernel),
                (0x13_2000, 0xffd_f000, Usable),
                (0xffd_f000, 0x1000_0000, Reserved),
                (0xb000_0000, 0xc000_0000, Reserved),
                (0xfd_0000_0000, 0x100_0000_0000, Reserved),
            ]
        );
        assert_eq!(dm.end(), 0x100_0000_0000);
        assert!(dm.covers(0, 0x1000_0000), "adjacent spans form one range");
        assert!(!dm.covers(0xfff_f000, 0x2000), "runs into the hole above low RAM");
        assert!(!dm.covers(0x2000_0000, 1));
        assert!(dm.covers(0xbfff_ffff, 1));
        assert!(!dm.covers(0xc000_0000, 1));
        assert_eq!(dm.find(0x20_0000).map(|s| s.kind), Some(Usable));
        assert_eq!(dm.find(0x1000_0000), None);
        assert_eq!(dm.find(u64::MAX), None);
    }

    #[test]
    fn unaligned_firmware_regions_are_rounded_out() {
        let m = map(&[(0x9fc00, 0xa0400, AcpiNvs)]);
        let dm = DirectMap::from_map(&m, u64::MAX);
        assert_eq!(spans(&dm), [(0x9f000, 0xa1000, AcpiNvs)]);
    }

    #[test]
    fn shared_page_goes_to_the_more_restrictive_kind() {
        // ACPI tables end half-way into a page that a reserved range starts in.
        let m = map(&[(0x1000, 0x2800, AcpiReclaimable), (0x2800, 0x4000, Reserved)]);
        let dm = DirectMap::from_map(&m, u64::MAX);
        assert_eq!(spans(&dm), [(0x1000, 0x2000, AcpiReclaimable), (0x2000, 0x4000, Reserved)]);

        // The other way round: the reserved range keeps the page.
        let m = map(&[(0x1000, 0x2800, Reserved), (0x2800, 0x4000, AcpiReclaimable)]);
        let dm = DirectMap::from_map(&m, u64::MAX);
        assert_eq!(spans(&dm), [(0x1000, 0x3000, Reserved), (0x3000, 0x4000, AcpiReclaimable)]);

        // A region entirely inside the shared page.
        let m = map(&[(0x1000, 0x1400, AcpiReclaimable), (0x1400, 0x1800, Reserved), (0x1800, 0x1c00, AcpiNvs)]);
        let dm = DirectMap::from_map(&m, u64::MAX);
        assert_eq!(spans(&dm), [(0x1000, 0x2000, AcpiNvs)]);
        assert!(dm.covers(0x1000, 0x1000));
    }

    #[test]
    fn bad_memory_is_left_out() {
        let m = map(&[(0, 0x4000, Usable), (0x4000, 0x5000, Bad), (0x5000, 0x8000, Usable)]);
        let dm = DirectMap::from_map(&m, u64::MAX);
        assert_eq!(spans(&dm), [(0, 0x4000, Usable), (0x5000, 0x8000, Usable)]);
        assert!(!dm.covers(0x3000, 0x2000));
    }

    #[test]
    fn same_kind_neighbours_merge_after_rounding() {
        let m = map(&[(0x1000, 0x1800, Reserved), (0x1800, 0x1c00, AcpiNvs), (0x1c00, 0x3000, Reserved)]);
        let dm = DirectMap::from_map(&m, u64::MAX);
        // The shared page is AcpiNvs-or-Reserved; AcpiNvs is more restrictive.
        assert_eq!(spans(&dm), [(0x1000, 0x2000, AcpiNvs), (0x2000, 0x3000, Reserved)]);
        let m = map(&[(0x1000, 0x1800, Reserved), (0x1800, 0x1c00, AcpiReclaimable), (0x1c00, 0x3000, Reserved)]);
        let dm = DirectMap::from_map(&m, u64::MAX);
        assert_eq!(spans(&dm), [(0x1000, 0x3000, Reserved)]);
    }

    #[test]
    fn limit_clips_the_map() {
        let m = map(&[(0, 0x10_0000, Usable), (0x20_0000, 0x40_0000, Reserved)]);
        let dm = DirectMap::from_map(&m, 0x30_0800);
        assert_eq!(spans(&dm), [(0, 0x10_0000, Usable), (0x20_0000, 0x30_0000, Reserved)]);
        assert!(DirectMap::from_map(&m, 0).spans().is_empty());
    }

    #[test]
    fn covers_edge_cases() {
        let dm = DirectMap::from_map(&map(&[(0x1000, 0x3000, Usable)]), u64::MAX);
        assert!(dm.covers(0x1000, 0));
        assert!(!dm.covers(0x3000, 0));
        assert!(dm.covers(0x2fff, 1));
        assert!(!dm.covers(0x2fff, 2));
        assert!(!dm.covers(0xfff, 2));
        assert!(!dm.covers(u64::MAX, 2), "overflowing range");
        assert!(!DirectMap::new().covers(0, 1));
        assert_eq!(DirectMap::new().end(), 0);
    }
}
