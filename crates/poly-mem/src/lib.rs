//! Physical memory map handling.
//!
//! Firmware memory maps (E820, UEFI) can be unsorted, overlapping and not
//! page-aligned. [`MemoryMap::normalized`] turns one into a sorted list of
//! non-overlapping regions where the most restrictive type wins wherever two
//! entries overlap, and usable RAM is trimmed inward to whole pages. Nothing
//! here allocates, so the kernel can use it before it has a heap.

#![no_std]

pub const PAGE_SIZE: u64 = 4096;

pub const fn align_up(x: u64, a: u64) -> u64 {
    match x.checked_add(a - 1) {
        Some(v) => v & !(a - 1),
        None => !(a - 1),
    }
}

pub const fn align_down(x: u64, a: u64) -> u64 {
    x & !(a - 1)
}

/// Region types, ordered from least to most restrictive. When regions
/// overlap, the higher variant wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RegionKind {
    Usable,
    /// ACPI tables; reusable once they have been parsed.
    AcpiReclaimable,
    /// Kernel image, boot structures, anything we placed ourselves.
    Kernel,
    Reserved,
    AcpiNvs,
    Bad,
}

impl RegionKind {
    /// Map an E820 / PVH memory map type.
    pub const fn from_e820(ty: u32) -> RegionKind {
        match ty {
            1 => RegionKind::Usable,
            3 => RegionKind::AcpiReclaimable,
            4 => RegionKind::AcpiNvs,
            5 => RegionKind::Bad,
            _ => RegionKind::Reserved,
        }
    }
}

/// A half-open physical range `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub start: u64,
    pub end: u64,
    pub kind: RegionKind,
}

impl Region {
    pub const fn new(start: u64, end: u64, kind: RegionKind) -> Region {
        Region { start, end, kind }
    }

    pub const fn len(&self) -> u64 {
        self.end - self.start
    }

    pub const fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Full;

/// A fixed-capacity memory map.
#[derive(Clone, Debug)]
pub struct MemoryMap {
    regions: [Region; Self::CAPACITY],
    len: usize,
}

impl Default for MemoryMap {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryMap {
    pub const CAPACITY: usize = 128;

    pub const fn new() -> MemoryMap {
        MemoryMap { regions: [Region::new(0, 0, RegionKind::Reserved); Self::CAPACITY], len: 0 }
    }

    /// Add a region. Empty and inverted ranges are ignored.
    pub fn push(&mut self, r: Region) -> Result<(), Full> {
        if r.is_empty() {
            return Ok(());
        }
        let slot = self.regions.get_mut(self.len).ok_or(Full)?;
        *slot = r;
        self.len += 1;
        Ok(())
    }

    pub fn regions(&self) -> &[Region] {
        &self.regions[..self.len]
    }

    pub fn total(&self, kind: RegionKind) -> u64 {
        self.regions().iter().filter(|r| r.kind == kind).map(Region::len).sum()
    }

    /// Resolve overlaps, sort, merge neighbours and page-align usable RAM.
    pub fn normalized(&self) -> Result<MemoryMap, Full> {
        // Every start and end is a boundary; between two neighbouring
        // boundaries the type is constant.
        let mut points = [0u64; 2 * Self::CAPACITY];
        let mut n = 0;
        for r in self.regions() {
            points[n] = r.start;
            points[n + 1] = r.end;
            n += 2;
        }
        let points = &mut points[..n];
        points.sort_unstable();

        let mut merged = MemoryMap::new();
        let mut prev: Option<u64> = None;
        for &p in points.iter() {
            if let Some(a) = prev
                && a != p
                && let Some(kind) = self.kind_over(a, p)
            {
                merged.push_merging(Region::new(a, p, kind))?;
            }
            prev = Some(p);
        }

        let mut out = MemoryMap::new();
        for r in merged.regions() {
            let r = if r.kind == RegionKind::Usable {
                Region::new(align_up(r.start, PAGE_SIZE), align_down(r.end, PAGE_SIZE), r.kind)
            } else {
                *r
            };
            out.push(r)?;
        }
        Ok(out)
    }

    /// Most restrictive type covering all of `[a, b)`, if any region does.
    fn kind_over(&self, a: u64, b: u64) -> Option<RegionKind> {
        self.regions().iter().filter(|r| r.start <= a && r.end >= b).map(|r| r.kind).max()
    }

    fn push_merging(&mut self, r: Region) -> Result<(), Full> {
        if let Some(last) = self.len.checked_sub(1).map(|i| &mut self.regions[i])
            && last.end == r.start
            && last.kind == r.kind
        {
            last.end = r.end;
            return Ok(());
        }
        self.push(r)
    }
}

/// Hands out physical frames from usable RAM, front to back, and never
/// frees. Used only until the real allocator is up.
#[derive(Debug)]
pub struct BumpFrameAllocator<'a> {
    map: &'a MemoryMap,
    region: usize,
    next: u64,
    allocated: u64,
}

impl<'a> BumpFrameAllocator<'a> {
    /// `map` must be normalized.
    pub fn new(map: &'a MemoryMap) -> Self {
        BumpFrameAllocator { map, region: 0, next: 0, allocated: 0 }
    }

    /// Physical address of a fresh 4 KiB frame.
    pub fn allocate(&mut self) -> Option<u64> {
        while let Some(r) = self.map.regions().get(self.region) {
            if r.kind == RegionKind::Usable {
                let frame = self.next.max(r.start);
                if frame + PAGE_SIZE <= r.end {
                    self.next = frame + PAGE_SIZE;
                    self.allocated += 1;
                    return Some(frame);
                }
            }
            self.region += 1;
        }
        None
    }

    pub fn allocated(&self) -> u64 {
        self.allocated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RegionKind::*;

    fn map(rs: &[(u64, u64, RegionKind)]) -> MemoryMap {
        let mut m = MemoryMap::new();
        for &(s, e, k) in rs {
            m.push(Region::new(s, e, k)).unwrap();
        }
        m
    }

    #[test]
    fn sorts_and_merges() {
        let n = map(&[(0x2000, 0x3000, Usable), (0x1000, 0x2000, Usable)]).normalized().unwrap();
        assert_eq!(n.regions(), &[Region::new(0x1000, 0x3000, Usable)]);
    }

    #[test]
    fn restrictive_type_wins_overlap() {
        let n = map(&[(0, 0x10000, Usable), (0x4000, 0x6000, Reserved)]).normalized().unwrap();
        assert_eq!(
            n.regions(),
            &[
                Region::new(0, 0x4000, Usable),
                Region::new(0x4000, 0x6000, Reserved),
                Region::new(0x6000, 0x10000, Usable),
            ]
        );
    }

    #[test]
    fn usable_is_trimmed_to_pages() {
        let n = map(&[(0x1001, 0x3fff, Usable)]).normalized().unwrap();
        assert_eq!(n.regions(), &[Region::new(0x2000, 0x3000, Usable)]);
        let n = map(&[(0x1001, 0x1fff, Usable)]).normalized().unwrap();
        assert!(n.regions().is_empty());
    }

    #[test]
    fn reservation_carves_kernel_out() {
        let n = map(&[(0, 0x80_0000, Usable), (0x10_0000, 0x20_0000, Kernel)]).normalized().unwrap();
        assert_eq!(n.total(Usable), 0x80_0000 - 0x10_0000);
        assert_eq!(n.total(Kernel), 0x10_0000);
    }

    #[test]
    fn empty_regions_are_ignored() {
        let m = map(&[(5, 5, Usable), (9, 3, Usable)]);
        assert!(m.regions().is_empty());
    }

    #[test]
    fn push_reports_full() {
        let mut m = MemoryMap::new();
        for i in 0..MemoryMap::CAPACITY as u64 {
            m.push(Region::new(i * 2, i * 2 + 1, Usable)).unwrap();
        }
        assert_eq!(m.push(Region::new(1000, 1001, Usable)), Err(Full));
    }

    #[test]
    fn bump_allocator_skips_holes_and_ends() {
        let n = map(&[(0x1000, 0x3000, Usable), (0x3000, 0x5000, Reserved), (0x5000, 0x6000, Usable)])
            .normalized()
            .unwrap();
        let mut a = BumpFrameAllocator::new(&n);
        assert_eq!(a.allocate(), Some(0x1000));
        assert_eq!(a.allocate(), Some(0x2000));
        assert_eq!(a.allocate(), Some(0x5000));
        assert_eq!(a.allocate(), None);
        assert_eq!(a.allocated(), 3);
    }

    #[test]
    fn align_up_saturates() {
        assert_eq!(align_up(u64::MAX, PAGE_SIZE), !(PAGE_SIZE - 1));
    }
}
