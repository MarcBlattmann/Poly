//! Randomized check of [`VmMap`] against a reference model that keeps one
//! entry per page and does everything the slow, obvious way.
//!
//! After every operation the region map must be well formed (sorted,
//! non-overlapping, page-aligned, inside the layout, fully merged, within the
//! map count) and describe exactly the model's pages; every result (address
//! chosen, error, replaced or removed pages, break, fault action) must be
//! the one the model predicts.

extern crate std;

use super::*;
use std::collections::BTreeSet;
use std::string::String;
use std::vec;

const P: u64 = PAGE_SIZE;
/// Pages in the test address space: small, so that it fills up, fragments
/// and runs into its edges often.
const N: u64 = 192;
const MIN: u64 = 0x10_0000;
const MAX: u64 = MIN + N * P;

/// xorshift64*: deterministic and dependency-free.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// What one page of the model holds. `offset` is the file offset of this
/// page itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Page {
    prot: Prot,
    max_prot: Prot,
    kind: Kind,
    file: Option<(u32, u64, bool)>,
}

impl Page {
    /// Whether `next`, on the following page, belongs to the same region.
    fn continues_into(&self, next: &Page) -> bool {
        self.prot == next.prot
            && self.max_prot == next.max_prot
            && self.kind == next.kind
            && match (self.file, next.file) {
                (None, None) => true,
                (Some((f, o, s)), Some((g, p, t))) => f == g && s == t && o.checked_add(P) == Some(p),
                _ => false,
            }
    }

    fn of(v: &Vma<u32>, addr: u64) -> Page {
        let file = match v.backing {
            Backing::Anonymous => None,
            Backing::File { file, offset, shared } => Some((file, offset + (addr - v.start), shared)),
        };
        Page { prot: v.prot, max_prot: v.max_prot, kind: v.kind, file }
    }
}

#[derive(Clone)]
struct Model {
    layout: Layout,
    pages: Vec<Option<Page>>,
    brk: Option<(u64, u64)>,
}

fn idx(addr: u64) -> usize {
    ((addr - MIN) / P) as usize
}

fn addr_of(i: usize) -> u64 {
    MIN + i as u64 * P
}

/// Number of regions the pages make: maximal runs of pages that continue
/// into each other.
fn runs(pages: &[Option<Page>]) -> usize {
    let mut n = 0;
    for i in 0..pages.len() {
        if let Some(p) = &pages[i] {
            let continued = i > 0 && pages[i - 1].as_ref().is_some_and(|q| q.continues_into(p));
            if !continued {
                n += 1;
            }
        }
    }
    n
}

impl Model {
    /// Pages that a new mapping may not use: mapped ones, and the guard gap
    /// below the start of every stack region.
    fn blocked(&self) -> Vec<bool> {
        let mut b: Vec<bool> = self.pages.iter().map(Option::is_some).collect();
        let gap = (self.layout.stack_guard_gap / P) as usize;
        for i in 0..self.pages.len() {
            let Some(p) = &self.pages[i] else { continue };
            let starts_region = i == 0 || !self.pages[i - 1].as_ref().is_some_and(|q| q.continues_into(p));
            if p.kind == Kind::Stack && starts_region {
                for slot in b.iter_mut().take(i).skip(i.saturating_sub(gap)) {
                    *slot = true;
                }
            }
        }
        b
    }

    /// `free[i]`: how many unblocked pages start at page `i`.
    fn free_runs(&self) -> Vec<u64> {
        let b = self.blocked();
        let mut free = vec![0; b.len() + 1];
        for i in (0..b.len()).rev() {
            free[i] = if b[i] { 0 } else { free[i + 1] + 1 };
        }
        free
    }

    /// Where a search for `len` bytes with `hint` lands.
    fn place(&self, hint: u64, len: u64) -> Option<u64> {
        let free = self.free_runs();
        let fits = |a: u64| a >= MIN && a.checked_add(len).is_some_and(|e| e <= MAX) && free[idx(a)] >= len / P;
        if let Some(h) = page_up(hint)
            && h != 0
            && fits(h.max(MIN))
        {
            return Some(h.max(MIN));
        }
        let base = self.layout.mmap_base;
        let below = |limit: u64| (0..N as usize).rev().map(addr_of).find(|&a| a + len <= limit && fits(a));
        let above = |floor: u64| (0..N as usize).map(addr_of).find(|&a| a >= floor && fits(a));
        match self.layout.search {
            Search::TopDown => below(base).or_else(|| above(MIN)),
            Search::BottomUp => above(base).or_else(|| below(MAX)),
        }
    }

    /// The pages of `[start, end)` (clamped to the space) that are mapped.
    fn mapped_in(&self, start: u64, end: u64) -> Vec<(u64, Page)> {
        let (s, e) = (start.clamp(MIN, MAX), end.clamp(MIN, MAX));
        (idx(s)..idx(e)).filter_map(|i| self.pages[i].map(|p| (addr_of(i), p))).collect()
    }

    /// `pages` if they respect the map count limit.
    fn within_limit(&self, pages: Vec<Option<Page>>) -> Result<Vec<Option<Page>>, Error> {
        if runs(&pages) > self.layout.max_map_count { Err(Error::NoMemory) } else { Ok(pages) }
    }

    /// The pages with `[start, end)` set to `page` (or cleared).
    fn with(&self, start: u64, end: u64, page: Option<Page>) -> Vec<Option<Page>> {
        let mut pages = self.pages.clone();
        let (s, e) = (start.clamp(MIN, MAX), end.clamp(MIN, MAX));
        for (i, slot) in pages.iter_mut().enumerate().take(idx(e)).skip(idx(s)) {
            *slot = page.map(|p| Page { file: p.file.map(|(f, o, sh)| (f, o + (addr_of(i) - start), sh)), ..p });
        }
        pages
    }

    fn map(&mut self, req: &MapRequest<u32>) -> Result<(u64, Vec<(u64, Page)>), Error> {
        if req.len == 0 {
            return Err(Error::Invalid);
        }
        if !req.max_prot.contains(req.prot) {
            return Err(Error::Access);
        }
        let file = match req.backing {
            Backing::Anonymous => None,
            Backing::File { file, offset, shared } => Some((file, offset, shared)),
        };
        if file.is_some_and(|(_, o, _)| !is_page_aligned(o)) {
            return Err(Error::Invalid);
        }
        let len = page_up(req.len).ok_or(Error::NoMemory)?;
        if len > MAX - MIN {
            return Err(Error::NoMemory);
        }
        if file.is_some_and(|(_, o, _)| o.checked_add(len).is_none()) {
            return Err(Error::Overflow);
        }
        let addr = match req.placement {
            Placement::Fixed(a) | Placement::FixedNoReplace(a) => {
                if !is_page_aligned(a) {
                    return Err(Error::Invalid);
                }
                if a < MIN {
                    return Err(Error::Permission);
                }
                if a.checked_add(len).is_none_or(|e| e > MAX) {
                    return Err(Error::NoMemory);
                }
                if matches!(req.placement, Placement::FixedNoReplace(_)) && !self.mapped_in(a, a + len).is_empty() {
                    return Err(Error::Exists);
                }
                a
            }
            Placement::Hint(h) => self.place(h, len).ok_or(Error::NoMemory)?,
        };
        let page = Page { prot: req.prot, max_prot: req.max_prot, kind: Kind::Normal, file };
        let replaced = self.mapped_in(addr, addr + len);
        self.pages = self.within_limit(self.with(addr, addr + len, Some(page)))?;
        Ok((addr, replaced))
    }

    fn unmap(&mut self, addr: u64, len: u64) -> Result<Vec<(u64, Page)>, Error> {
        if !is_page_aligned(addr) || len == 0 {
            return Err(Error::Invalid);
        }
        let end = page_up(len).and_then(|l| addr.checked_add(l)).ok_or(Error::Invalid)?;
        if end > MAX {
            return Err(Error::Invalid);
        }
        let removed = self.mapped_in(addr, end);
        self.pages = self.within_limit(self.with(addr, end, None))?;
        Ok(removed)
    }

    fn protect(&mut self, addr: u64, len: u64, prot: Prot) -> Result<(), Error> {
        if !is_page_aligned(addr) {
            return Err(Error::Invalid);
        }
        if len == 0 {
            return Ok(());
        }
        let end = page_up(len).and_then(|l| addr.checked_add(l)).ok_or(Error::NoMemory)?;
        if end > MAX {
            return Err(Error::NoMemory);
        }
        let mapped = self.mapped_in(addr, end);
        if addr < MIN || mapped.len() as u64 != (end - addr) / P {
            return Err(Error::NoMemory);
        }
        if mapped.iter().any(|(_, p)| !p.max_prot.contains(prot)) {
            return Err(Error::Access);
        }
        let mut pages = self.pages.clone();
        for (a, p) in mapped {
            pages[idx(a)] = Some(Page { prot, ..p });
        }
        self.pages = self.within_limit(pages)?;
        Ok(())
    }

    fn brk(&mut self, new: u64) -> (u64, Vec<(u64, Page)>) {
        let Some((start, current)) = self.brk else { return (0, vec![]) };
        let unchanged = (current, vec![]);
        if new < start || new - start > self.layout.heap_max {
            return unchanged;
        }
        let (old_end, Some(new_end)) = (page_up(current).unwrap(), page_up(new)) else { return unchanged };
        if new_end > MAX {
            return unchanged;
        }
        let mut removed = vec![];
        if new_end < old_end {
            removed = self.mapped_in(new_end, old_end);
            match self.within_limit(self.with(new_end, old_end, None)) {
                Ok(pages) => self.pages = pages,
                Err(_) => return unchanged,
            }
        } else if new_end > old_end {
            let blocked = self.blocked();
            if (idx(old_end)..idx(new_end)).any(|i| blocked[i]) {
                return unchanged;
            }
            let heap = Page { prot: Prot::RW, max_prot: Prot::ALL, kind: Kind::Heap, file: None };
            match self.within_limit(self.with(old_end, new_end, Some(heap))) {
                Ok(pages) => self.pages = pages,
                Err(_) => return unchanged,
            }
        }
        self.brk = Some((start, new));
        (new, removed)
    }

    fn fault(&mut self, addr: u64, access: Access) -> FaultAction<u32> {
        if !(MIN..MAX).contains(&addr) {
            return FaultAction::Segv(SegvCode::MapErr);
        }
        let i = idx(addr);
        let page = match self.pages[i] {
            Some(p) => p,
            None => {
                let above = (i + 1..N as usize).find(|&j| self.pages[j].is_some());
                let Some(above) = above.filter(|&j| self.pages[j].unwrap().kind == Kind::Stack) else {
                    return FaultAction::Segv(SegvCode::MapErr);
                };
                if addr_of(i) < self.layout.stack_floor() {
                    return FaultAction::Segv(SegvCode::MapErr);
                }
                let below = (0..i).rev().find(|&j| self.pages[j].is_some());
                if below.is_some_and(|j| addr_of(j + 1) + self.layout.stack_guard_gap > addr_of(i)) {
                    return FaultAction::Segv(SegvCode::MapErr);
                }
                let stack = self.pages[above].unwrap();
                if !stack.prot.allows(access) {
                    return FaultAction::Segv(SegvCode::AccErr);
                }
                for slot in &mut self.pages[i..above] {
                    *slot = Some(stack);
                }
                stack
            }
        };
        if !page.prot.allows(access) {
            return FaultAction::Segv(SegvCode::AccErr);
        }
        let page_addr = page_down(addr);
        match page.file {
            None => FaultAction::ZeroFill { page: page_addr, prot: page.prot },
            Some((file, offset, shared)) => {
                let private_copy = !shared && access == Access::Write;
                let prot = if shared || private_copy { page.prot } else { write_protected(page.prot) };
                FaultAction::ReadFile { page: page_addr, file, offset, prot, private_copy }
            }
        }
    }
}

/// Every page of the regions, in order.
fn pages_of(regions: &[Vma<u32>]) -> Vec<(u64, Page)> {
    let mut out = vec![];
    for v in regions {
        let mut a = v.start;
        while a < v.end {
            out.push((a, Page::of(v, a)));
            a += P;
        }
    }
    out
}

/// The map is well formed and holds exactly the model's pages.
fn check(m: &VmMap<u32>, model: &Model, what: &str) {
    let l = m.layout();
    let mut prev: Option<&Vma<u32>> = None;
    for (k, v) in &m.vmas {
        assert_eq!(*k, v.start, "{what}: key");
        assert!(v.start < v.end, "{what}: empty region {v:?}");
        assert!(is_page_aligned(v.start) && is_page_aligned(v.end), "{what}: unaligned {v:?}");
        assert!(l.min_addr <= v.start && v.end <= l.max_addr, "{what}: outside the space {v:?}");
        assert!(v.max_prot.contains(v.prot), "{what}: prot above max_prot {v:?}");
        if let Some(p) = prev {
            assert!(p.end <= v.start, "{what}: overlap {p:?} {v:?}");
            assert!(!p.can_merge(v), "{what}: not merged {p:?} {v:?}");
        }
        prev = Some(v);
    }
    assert!(m.region_count() <= l.max_map_count, "{what}: too many regions");
    let regions: Vec<Vma<u32>> = m.regions().cloned().collect();
    let expected: Vec<(u64, Page)> = model.mapped_in(MIN, MAX);
    assert_eq!(pages_of(&regions), expected, "{what}: pages differ");
    assert_eq!(m.region_count(), runs(&model.pages), "{what}: region count");
    assert_eq!(m.brk.map(|b| (b.start, b.current)), model.brk, "{what}: break");
}

fn rand_addr(rng: &mut Rng) -> u64 {
    match rng.below(20) {
        0 => 0,
        1 => rng.next(),
        2 => MIN + rng.below(N) * P + 1 + rng.below(P - 1),
        3 => MAX - rng.below(3) * P,
        _ => MIN - 2 * P + rng.below(N + 4) * P,
    }
}

fn rand_len(rng: &mut Rng) -> u64 {
    match rng.below(30) {
        0 => 0,
        1 => u64::MAX - rng.below(2 * P),
        2 => rng.below(N + 4) * P + 1 + rng.below(P - 1),
        3 => (N - rng.below(4)) * P,
        4..=8 => (1 + rng.below(48)) * P,
        _ => (1 + rng.below(8)) * P,
    }
}

fn rand_prot(rng: &mut Rng) -> Prot {
    // Mostly a few common protections, so that regions merge often.
    match rng.below(6) {
        0 => Prot::from_bits(rng.below(8)).unwrap(),
        1 => Prot::READ,
        _ => Prot::RW,
    }
}

fn rand_request(rng: &mut Rng) -> MapRequest<u32> {
    let len = rand_len(rng);
    let prot = rand_prot(rng);
    let mut req = if rng.chance(55) {
        MapRequest::anonymous(len, prot)
    } else {
        let offset = match rng.below(25) {
            0 => 1 + rng.below(P - 1),
            1 => page_down(u64::MAX) - rng.below(4) * P,
            _ => rng.below(16) * P,
        };
        MapRequest::file(len, prot, rng.below(3) as u32, offset, rng.chance(30))
    };
    if rng.chance(15) {
        req =
            req.with_max_prot(Prot::from_bits(rng.below(8)).unwrap() | if rng.chance(70) { prot } else { Prot::NONE });
    }
    let placement = match rng.below(20) {
        0..=7 => Placement::Hint(0),
        8..=10 => Placement::Hint(rand_addr(rng)),
        11..=16 => Placement::Fixed(rand_addr(rng)),
        _ => Placement::FixedNoReplace(rand_addr(rng)),
    };
    req.at(placement)
}

/// Run `steps` random operations; `seen` collects which outcomes occurred.
fn run(seed: u64, steps: usize, seen: &mut BTreeSet<String>) {
    let mut rng = Rng(seed | 1);
    let layout = Layout {
        min_addr: MIN,
        max_addr: MAX,
        stack_top: MAX - 16 * P,
        stack_max: 24 * P,
        stack_guard_gap: [0, P, 3 * P][rng.below(3) as usize],
        mmap_base: MIN + (60 + rng.below(100)) * P,
        search: if rng.chance(30) { Search::BottomUp } else { Search::TopDown },
        max_map_count: if rng.chance(50) { 6 + rng.below(10) as usize } else { 1000 },
        heap_max: (20 + rng.below(60)) * P,
    };
    let mut m: VmMap<u32> = VmMap::new(layout).unwrap();
    let mut model = Model { layout, pages: vec![None; N as usize], brk: None };

    // The usual start of a process: a stack, then a program break.
    let bottom = m.setup_stack(4 * P, Prot::RW).unwrap();
    model.pages = model.with(
        bottom,
        layout.stack_top,
        Some(Page { prot: Prot::RW, max_prot: Prot::ALL, kind: Kind::Stack, file: None }),
    );
    let brk_start = MIN + rng.below(40) * P + rng.below(P);
    m.init_brk(brk_start).unwrap();
    model.brk = Some((brk_start, brk_start));
    check(&m, &model, "setup");

    let mut counts = [0usize; 5];
    for step in 0..steps {
        let op = rng.below(100);
        let what;
        if op < 30 {
            let req = rand_request(&mut rng);
            what = std::format!("{seed}/{step}: map {req:?}");
            let got = m.map(req.clone());
            let want = model.map(&req);
            match (&got, &want) {
                (Ok(g), Ok((addr, replaced))) => {
                    assert_eq!(g.addr, *addr, "{what}");
                    assert_eq!(&pages_of(&g.replaced), replaced, "{what}: replaced");
                    counts[0] += 1;
                    if !replaced.is_empty() {
                        seen.insert("map replaced".into());
                    }
                }
                (Err(g), Err(w)) => {
                    assert_eq!(g, w, "{what}");
                    seen.insert(std::format!("map {g:?}"));
                }
                _ => panic!("{what}: got {got:?}, model says {want:?}"),
            }
        } else if op < 50 {
            let (addr, len) = (rand_addr(&mut rng), rand_len(&mut rng));
            what = std::format!("{seed}/{step}: unmap {addr:#x} {len:#x}");
            let got = m.unmap(addr, len).map(|r| pages_of(&r));
            assert_eq!(got, model.unmap(addr, len), "{what}");
            counts[1] += got.is_ok() as usize;
            match &got {
                Ok(removed) if !removed.is_empty() => seen.insert("unmap removed".into()),
                Ok(_) => seen.insert("unmap hole".into()),
                Err(e) => seen.insert(std::format!("unmap {e:?}")),
            };
        } else if op < 70 {
            let (addr, len, prot) = (rand_addr(&mut rng), rand_len(&mut rng), rand_prot(&mut rng));
            what = std::format!("{seed}/{step}: protect {addr:#x} {len:#x} {prot:?}");
            let got = m.protect(addr, len, prot);
            assert_eq!(got, model.protect(addr, len, prot), "{what}");
            counts[2] += got.is_ok() as usize;
            if let Err(e) = got {
                seen.insert(std::format!("protect {e:?}"));
            }
        } else if op < 82 {
            let new = match rng.below(10) {
                0 => 0,
                1 => rng.next(),
                2 => brk_start - rng.below(3) * P,
                _ => brk_start + rng.below(30) * P + rng.below(P),
            };
            what = std::format!("{seed}/{step}: brk {new:#x}");
            let got = m.brk(new);
            let (brk, removed) = model.brk(new);
            assert_eq!((got.brk, pages_of(&got.removed)), (brk, removed), "{what}");
            counts[3] += (brk == new) as usize;
            seen.insert(
                if brk != new {
                    "brk refused"
                } else if got.removed.is_empty() {
                    "brk grown"
                } else {
                    "brk shrunk"
                }
                .into(),
            );
        } else {
            let addr = match rng.below(4) {
                0 => rand_addr(&mut rng),
                _ => MIN + rng.below(N * P),
            };
            let access = [Access::Read, Access::Write, Access::Execute][rng.below(3) as usize];
            what = std::format!("{seed}/{step}: fault {addr:#x} {access:?}");
            let was_mapped = m.find(addr).is_some();
            let got = m.fault(addr, access, PageState::Absent);
            assert_eq!(got, model.fault(addr, access), "{what}");
            counts[4] += !matches!(got, FaultAction::Segv(_)) as usize;
            let outcome = match &got {
                FaultAction::Segv(code) => std::format!("fault {code:?}"),
                _ if !was_mapped => "fault grew the stack".into(),
                FaultAction::ReadFile { .. } => "fault file".into(),
                _ => "fault other".into(),
            };
            seen.insert(outcome);
        }
        check(&m, &model, &what);
    }
    // Every kind of operation succeeded a fair number of times.
    assert!(counts.iter().all(|&c| c > steps / 100), "seed {seed}: successes {counts:?}");
}

#[test]
fn random_operations_match_the_model() {
    let mut seen = BTreeSet::new();
    for seed in 1..=12u64 {
        run(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15), 2500, &mut seen);
    }
    // The operations reached every interesting outcome.
    let wanted = [
        "map replaced",
        "map Invalid",
        "map NoMemory",
        "map Exists",
        "map Access",
        "map Permission",
        "map Overflow",
        "unmap removed",
        "unmap hole",
        "unmap Invalid",
        "unmap NoMemory",
        "protect Invalid",
        "protect NoMemory",
        "protect Access",
        "brk grown",
        "brk shrunk",
        "brk refused",
        "fault MapErr",
        "fault AccErr",
        "fault grew the stack",
        "fault file",
        "fault other",
    ];
    let missing: Vec<&str> = wanted.iter().copied().filter(|w| !seen.contains(*w)).collect();
    assert!(missing.is_empty(), "never happened: {missing:?}");
}
