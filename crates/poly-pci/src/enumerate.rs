//! Finding the functions on the buses.
//!
//! On each bus, function 0 of every device is read first; functions 1-7 are
//! only looked at when function 0 has the multi-function bit set (PCI 3.0
//! §6.2.1), because single-function devices may answer on every function
//! number. Bus numbers are taken as firmware assigned them; nothing here
//! renumbers bridges.

use crate::{Bdf, ConfigSpace, Header};

/// Position within one bus: `slot` is `device * 8 + function`, 256 once the
/// bus is done.
#[derive(Clone, Copy)]
struct Cursor {
    bus: u8,
    slot: u16,
}

impl Cursor {
    const fn new(bus: u8) -> Cursor {
        Cursor { bus, slot: 0 }
    }

    /// Advance to the next function present on this bus.
    fn next<C: ConfigSpace + ?Sized>(&mut self, cfg: &C) -> Option<Header> {
        while self.slot < 256 {
            let bdf = Bdf::new(self.bus, (self.slot >> 3) as u8, (self.slot & 7) as u8);
            let header = Header::read(cfg, bdf);
            if bdf.function() == 0 {
                // No function 0 means no device; a single-function device
                // has no functions 1-7.
                let multifunction = header.is_some_and(|h| h.multifunction);
                self.slot += if multifunction { 1 } else { 8 };
            } else {
                self.slot += 1;
            }
            if header.is_some() {
                return header;
            }
        }
        None
    }
}

/// 256 bits, one per bus number.
struct BusSet([u64; 4]);

impl BusSet {
    /// Insert `bus`; false if it was already there.
    fn insert(&mut self, bus: u8) -> bool {
        let (word, bit) = ((bus >> 6) as usize, 1u64 << (bus & 63));
        let fresh = self.0[word] & bit == 0;
        self.0[word] |= bit;
        fresh
    }
}

/// Walk the hierarchy below `root_bus` depth-first, calling `visit` on every
/// function in bus / device / function order, with each bridge's buses
/// visited right after the bridge itself. Returns the number of functions
/// found.
///
/// A bridge is followed only if its configuration makes sense: the secondary
/// bus is above the bridge's own bus, not above its subordinate bus, not
/// above `last_bus` (the end of the ECAM window) and not already scanned.
/// Every bus is scanned at most once, so a looping or overlapping bridge
/// setup cannot make the walk run forever, and the explicit stack never
/// holds more than 256 entries (about 1 KiB), so it does not recurse.
pub fn scan<C: ConfigSpace + ?Sized>(cfg: &C, root_bus: u8, last_bus: u8, mut visit: impl FnMut(&Header)) -> usize {
    let mut seen = BusSet([0; 4]);
    let mut stack = [Cursor::new(0); 256];
    seen.insert(root_bus);
    stack[0] = Cursor::new(root_bus);
    let mut depth: usize = 1;
    let mut found = 0;
    while let Some(top) = depth.checked_sub(1).and_then(|i| stack.get_mut(i)) {
        let Some(header) = top.next(cfg) else {
            depth -= 1;
            continue;
        };
        visit(&header);
        found += 1;
        if let Some(b) = header.bridge
            && b.secondary > header.bdf.bus()
            && b.secondary <= b.subordinate
            && b.secondary <= last_bus
            && seen.insert(b.secondary)
            && let Some(slot) = stack.get_mut(depth)
        {
            *slot = Cursor::new(b.secondary);
            depth += 1;
        }
    }
    found
}

/// Brute force: look at every bus in `first_bus..=last_bus`, whether or not
/// a bridge leads there. Slower (every bus is probed), but finds root buses
/// of additional host bridges that no PCI-to-PCI bridge points at. Returns
/// the number of functions found.
pub fn scan_all<C: ConfigSpace + ?Sized>(
    cfg: &C,
    first_bus: u8,
    last_bus: u8,
    mut visit: impl FnMut(&Header),
) -> usize {
    let mut found = 0;
    for bus in first_bus..=last_bus {
        let mut cursor = Cursor::new(bus);
        while let Some(header) = cursor.next(cfg) {
            visit(&header);
            found += 1;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Function, Sim};
    extern crate std;
    use std::vec::Vec;

    fn bdf(b: u8, d: u8, f: u8) -> Bdf {
        Bdf::new(b, d, f)
    }

    /// Bus 0: host bridge, a bridge to buses 1-2, a multi-function device
    /// with a hole at function 1, a single-function device that (like some
    /// real ones) answers on every function number. Bus 1 holds a bridge to
    /// bus 2; bus 2 one endpoint. Bus 7 is only reachable by brute force.
    fn topology() -> Sim {
        let s = Sim::new();
        s.add(bdf(0, 0, 0), Function::endpoint(0x8086, 0x29c0, [6, 0, 0]));
        s.add(bdf(0, 1, 0), Function::bridge(0, 1, 2));
        s.add(bdf(0, 2, 0), Function::endpoint(0x10, 0x20, [1, 6, 1]).multifunction());
        s.add(bdf(0, 2, 2), Function::endpoint(0x10, 0x21, [1, 6, 1]));
        s.add(bdf(0, 2, 7), Function::endpoint(0x10, 0x22, [1, 6, 1]));
        for f in 0..8 {
            s.add(bdf(0, 3, f), Function::endpoint(0x30, 0x40, [2, 0, 0]));
        }
        s.add(bdf(1, 0, 0), Function::bridge(1, 2, 2));
        s.add(bdf(2, 3, 0), Function::endpoint(0x50, 0x60, [1, 8, 2]));
        s.add(bdf(7, 0, 0), Function::endpoint(0x70, 0x80, [3, 0, 0]));
        s
    }

    fn collect(f: impl FnOnce(&mut dyn FnMut(&Header)) -> usize) -> Vec<Bdf> {
        let mut v = Vec::new();
        let n = f(&mut |h: &Header| v.push(h.bdf));
        assert_eq!(n, v.len());
        v
    }

    #[test]
    fn follows_bridges_depth_first() {
        let s = topology();
        let found = collect(|v| scan(&s, 0, 255, v));
        assert_eq!(
            found,
            [
                bdf(0, 0, 0),
                bdf(0, 1, 0),
                bdf(1, 0, 0),
                bdf(2, 3, 0),
                bdf(0, 2, 0),
                bdf(0, 2, 2),
                bdf(0, 2, 7),
                bdf(0, 3, 0)
            ]
        );
    }

    #[test]
    fn brute_force_finds_unbridged_bus() {
        let s = topology();
        let found = collect(|v| scan_all(&s, 0, 255, v));
        assert_eq!(found.len(), 9);
        assert!(found.contains(&bdf(7, 0, 0)));
        assert_eq!(collect(|v| scan_all(&s, 1, 2, v)), [bdf(1, 0, 0), bdf(2, 3, 0)]);
    }

    #[test]
    fn last_bus_limits_following() {
        let s = topology();
        let found = collect(|v| scan(&s, 0, 1, v));
        assert!(found.contains(&bdf(1, 0, 0)));
        assert!(!found.contains(&bdf(2, 3, 0)));
    }

    #[test]
    fn bad_bridges_are_not_followed() {
        let s = Sim::new();
        // Secondary 0 (unconfigured), secondary below own bus, secondary
        // above subordinate, and two bridges claiming the same bus.
        s.add(bdf(0, 1, 0), Function::bridge(0, 0, 0));
        s.add(bdf(0, 2, 0), Function::bridge(0, 3, 3));
        s.add(bdf(0, 4, 0), Function::bridge(0, 5, 4));
        s.add(bdf(0, 5, 0), Function::bridge(0, 3, 3));
        s.add(bdf(3, 0, 0), Function::bridge(3, 1, 1));
        s.add(bdf(3, 1, 0), Function::bridge(3, 3, 3));
        s.add(bdf(3, 2, 0), Function::endpoint(1, 2, [0; 3]));
        s.add(bdf(5, 0, 0), Function::endpoint(1, 2, [0; 3]));
        let found = collect(|v| scan(&s, 0, 255, v));
        assert_eq!(
            found,
            [bdf(0, 1, 0), bdf(0, 2, 0), bdf(3, 0, 0), bdf(3, 1, 0), bdf(3, 2, 0), bdf(0, 4, 0), bdf(0, 5, 0)]
        );
    }

    #[test]
    fn every_bus_bridged_terminates() {
        // A bridge on every bus pointing at the next: the deepest possible
        // hierarchy. The stack must hold it without overflowing.
        let s = Sim::new();
        for b in 0..=255u8 {
            let next = b.wrapping_add(1);
            s.add(bdf(b, 0, 0), Function::bridge(b, next, 255));
        }
        assert_eq!(scan(&s, 0, 255, |_| {}), 256);
    }

    #[test]
    fn root_bus_need_not_be_zero() {
        let s = topology();
        assert_eq!(collect(|v| scan(&s, 7, 255, v)), [bdf(7, 0, 0)]);
        assert_eq!(scan(&s, 9, 255, |_| {}), 0);
    }
}
