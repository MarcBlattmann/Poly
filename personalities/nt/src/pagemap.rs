//! The state of the pages of an allocation as runs (ADR 0155).
//!
//! Chromium reserves address space in sizes of tens and hundreds of
//! gigabytes (the allocator's pools, V8's cage) and commits it a little at
//! a time. One entry per page would be 8 bytes per 4 KiB of a reservation:
//! 2 MiB per GiB, more than the kernel's heap hands out in one piece for a
//! 4 GiB reservation. A [`PageMap`] keeps a sorted list of runs of equal
//! state instead, so a reservation is one run and the cost grows with the
//! number of changes of state, not with the size.

use crate::vm::PageState;
use alloc::vec;
use alloc::vec::Vec;
use core::ops::Range;

/// The pages `0..len` as runs of equal state, in order, each starting where
/// the previous one ends, neighbours different.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageMap {
    len: usize,
    /// `(first page, state)`; the first run starts at 0.
    runs: Vec<(usize, PageState)>,
}

impl PageMap {
    /// `len` pages, all in `state`.
    pub fn new(len: usize, state: PageState) -> PageMap {
        PageMap { len, runs: if len == 0 { Vec::new() } else { vec![(0, state)] } }
    }

    /// The pages of a list, one state each.
    pub fn from_states(states: &[PageState]) -> PageMap {
        let mut map = PageMap { len: states.len(), runs: Vec::new() };
        for (i, &s) in states.iter().enumerate() {
            if map.runs.last().is_none_or(|&(_, last)| last != s) {
                map.runs.push((i, s));
            }
        }
        map
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The index of the run that holds page `i`.
    fn run_of(&self, i: usize) -> usize {
        self.runs.partition_point(|&(start, _)| start <= i) - 1
    }

    /// The state of page `i` (which must be inside).
    pub fn get(&self, i: usize) -> PageState {
        assert!(i < self.len, "page {i} of {}", self.len);
        self.runs[self.run_of(i)].1
    }

    /// The runs that overlap `range`, cut to it: `(pages, state)`.
    pub fn runs_in(&self, range: Range<usize>) -> impl Iterator<Item = (Range<usize>, PageState)> + '_ {
        let first = if range.is_empty() { self.runs.len() } else { self.run_of(range.start) };
        self.runs[first..].iter().enumerate().map_while(move |(k, &(start, state))| {
            let end = self.runs.get(first + k + 1).map_or(self.len, |&(next, _)| next);
            let (from, to) = (start.max(range.start), end.min(range.end));
            (from < to).then_some((from..to, state))
        })
    }

    /// How many pages from `i` on are in the state of page `i`.
    pub fn run_length_from(&self, i: usize) -> usize {
        let r = self.run_of(i);
        self.runs.get(r + 1).map_or(self.len, |&(next, _)| next) - i
    }

    /// Whether any page of `range` is in `state`.
    pub fn contains(&self, range: Range<usize>, state: &PageState) -> bool {
        self.runs_in(range).any(|(_, s)| s == *state)
    }

    /// Whether any page of `range` satisfies `pred`.
    pub fn any(&self, range: Range<usize>, pred: impl Fn(&PageState) -> bool) -> bool {
        self.runs_in(range).any(|(_, s)| pred(&s))
    }

    /// Set the pages of `range` to `state`.
    pub fn fill(&mut self, range: Range<usize>, state: PageState) {
        assert!(range.end <= self.len, "range {range:?} of {}", self.len);
        if range.is_empty() {
            return;
        }
        let before: Vec<(usize, PageState)> = self.runs.iter().copied().take_while(|&(s, _)| s < range.start).collect();
        // The state the run that crosses `range.end` goes on in.
        let after_state = self.get_or(range.end);
        let mut runs = before;
        runs.push((range.start, state));
        if range.end < self.len {
            // What follows: the rest of the run that holds `range.end`, then
            // the runs after it.
            runs.push((range.end, after_state.expect("inside")));
            runs.extend(self.runs.iter().copied().filter(|&(s, _)| s > range.end));
        }
        runs.dedup_by(|b, a| a.1 == b.1);
        self.runs = runs;
    }

    fn get_or(&self, i: usize) -> Option<PageState> {
        (i < self.len).then(|| self.get(i))
    }

    /// Set the pages of `range` that are in state `from` to `to`.
    pub fn replace(&mut self, range: Range<usize>, from: PageState, to: PageState) {
        let found: Vec<Range<usize>> = self.runs_in(range).filter(|&(_, s)| s == from).map(|(r, _)| r).collect();
        for r in found {
            self.fill(r, to);
        }
    }

    /// The number of runs (for tests and diagnostics).
    pub fn run_count(&self) -> usize {
        self.runs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protect::Protection;

    fn c(v: u32) -> PageState {
        PageState::Committed(Protection::new(v, true).unwrap())
    }

    const R: PageState = PageState::Reserved;

    /// A model: one state per page.
    fn flat(m: &PageMap) -> Vec<PageState> {
        (0..m.len()).map(|i| m.get(i)).collect()
    }

    #[test]
    fn a_big_reservation_is_one_run() {
        // 256 GiB of 4 KiB pages.
        let m = PageMap::new(64 << 20, R);
        assert_eq!((m.len(), m.run_count()), (64 << 20, 1));
        assert_eq!(m.run_length_from(5), (64 << 20) - 5);
    }

    #[test]
    fn fill_splits_and_merges_runs() {
        let mut m = PageMap::new(10, R);
        m.fill(3..6, c(0x04));
        assert_eq!(m.run_count(), 3);
        assert_eq!(flat(&m), [R, R, R, c(4), c(4), c(4), R, R, R, R]);
        // The same state next to a run joins it.
        m.fill(6..8, c(0x04));
        assert_eq!(m.run_count(), 3);
        m.fill(0..3, c(0x04));
        assert_eq!(m.run_count(), 2);
        // Back to one.
        m.fill(0..10, R);
        assert_eq!((m.run_count(), m.get(9)), (1, R));
        // The ends.
        m.fill(0..1, c(2));
        m.fill(9..10, c(2));
        assert_eq!(flat(&m), [c(2), R, R, R, R, R, R, R, R, c(2)]);
        // Inside one run, and an empty range.
        m.fill(4..4, c(4));
        m.fill(4..5, c(4));
        assert_eq!(flat(&m)[3..6], [R, c(4), R]);
    }

    #[test]
    fn queries_cut_to_the_range() {
        let m = PageMap::from_states(&[R, R, c(4), c(4), c(2), R]);
        assert_eq!(m.run_count(), 4);
        let runs: Vec<_> = m.runs_in(1..5).collect();
        assert_eq!(runs, [(1..2, R), (2..4, c(4)), (4..5, c(2))]);
        assert!(m.contains(0..6, &c(2)) && !m.contains(0..4, &c(2)));
        assert!(m.any(2..4, |s| *s == c(4)) && !m.any(0..2, |s| *s != R));
        assert_eq!(m.run_length_from(2), 2);
        assert_eq!(m.runs_in(3..3).count(), 0);
    }

    #[test]
    fn replace_changes_only_the_pages_in_one_state() {
        let mut m = PageMap::from_states(&[R, c(4), R, R, c(2), R]);
        m.replace(0..6, R, c(0x40));
        assert_eq!(flat(&m), [c(0x40), c(4), c(0x40), c(0x40), c(2), c(0x40)]);
        m.replace(1..3, c(0x40), R);
        assert_eq!(flat(&m), [c(0x40), c(4), R, c(0x40), c(2), c(0x40)]);
    }

    #[test]
    fn fill_agrees_with_a_flat_list_for_many_ranges() {
        let states = [R, c(0x20), c(2), c(4), c(0x40)];
        let mut seed = 7u32;
        let mut next = move |n: usize| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 8) as usize % n
        };
        let mut m = PageMap::new(40, R);
        let mut model = vec![R; 40];
        for _ in 0..400 {
            let (a, b) = (next(41), next(41));
            let (a, b) = (a.min(b), a.max(b));
            let s = states[next(states.len())];
            m.fill(a..b, s);
            model[a..b].fill(s);
            assert_eq!(flat(&m), model);
            // Neighbouring runs always differ.
            for w in m.runs.windows(2) {
                assert_ne!(w[0].1, w[1].1);
            }
        }
    }
}
