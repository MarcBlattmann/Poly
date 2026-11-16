//! The unload event trace (`RtlGetUnloadEventTrace`, `RtlGetUnloadEventTraceEx`): the record of the
//! last libraries a process unloaded, kept in a circular array of [`ELEMENT_COUNT`] elements that
//! crash reporters read to tell which modules were gone before a fault (Chrome's crash handler
//! resolves `RtlGetUnloadEventTraceEx` by name and calls it; without the export it calls a null
//! pointer, specs/windows/apps/chrome/s2-interception.md).
//!
//! The element is `RTL_UNLOAD_EVENT_TRACE` (Learn: "RTL_UNLOAD_EVENT_TRACE structure" and
//! `RtlGetUnloadEventTraceEx`): the module's base and size, a sequence number, the time stamp and
//! check sum of its header, its name (at most 31 characters and a NUL) and a two-word version.

/// Elements in the array (`RTL_UNLOAD_EVENT_TRACE_NUMBER`).
pub const ELEMENT_COUNT: usize = 64;
/// Characters in a name, with the NUL.
pub const NAME_UNITS: usize = 32;

/// `RTL_UNLOAD_EVENT_TRACE`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Element {
    pub base_address: u64,
    pub size_of_image: u64,
    pub sequence: u32,
    pub time_date_stamp: u32,
    pub check_sum: u32,
    pub image_name: [u16; NAME_UNITS],
    pub version: [u32; 2],
}

/// `sizeof(RTL_UNLOAD_EVENT_TRACE)` (what `ElementSize` points to).
pub const ELEMENT_SIZE: u32 = core::mem::size_of::<Element>() as u32;

/// The circular array and the sequence number of the next event.
#[derive(Clone, Debug)]
pub struct Trace {
    pub elements: [Element; ELEMENT_COUNT],
    next: u32,
}

impl Trace {
    pub const fn new() -> Trace {
        const E: Element = Element {
            base_address: 0,
            size_of_image: 0,
            sequence: 0,
            time_date_stamp: 0,
            check_sum: 0,
            image_name: [0; NAME_UNITS],
            version: [0; 2],
        };
        Trace { elements: [E; ELEMENT_COUNT], next: 1 }
    }

    /// Record the unload of the module at `base` (`size` bytes, called `name`: cut to 31
    /// characters). The element is the one the sequence number picks (`(sequence - 1)` modulo the
    /// count), so the 65th event replaces the first.
    pub fn record(&mut self, base: u64, size: u64, name: &[u16], time_date_stamp: u32, check_sum: u32) {
        let sequence = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        let slot = (sequence.wrapping_sub(1) as usize) % ELEMENT_COUNT;
        let mut image_name = [0u16; NAME_UNITS];
        for (d, s) in image_name.iter_mut().zip(name.iter().take(NAME_UNITS - 1)) {
            *d = *s;
        }
        self.elements[slot] = Element {
            base_address: base,
            size_of_image: size,
            sequence,
            time_date_stamp,
            check_sum,
            image_name,
            version: [0; 2],
        };
    }
}

impl Default for Trace {
    fn default() -> Self {
        Trace::new()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn the_element_has_the_layout_of_rtl_unload_event_trace() {
        // BaseAddress, SizeOfImage, Sequence, TimeDateStamp, CheckSum, ImageName[32], Version[2], padded to 8.
        assert_eq!(ELEMENT_SIZE, 104);
        assert_eq!(core::mem::offset_of!(Element, sequence), 16);
        assert_eq!(core::mem::offset_of!(Element, image_name), 28);
        assert_eq!(core::mem::offset_of!(Element, version), 92);
    }

    #[test]
    fn events_are_numbered_from_one_and_kept_in_order() {
        let mut t = Trace::new();
        t.record(0x1000_0000, 0x5000, &wide("a.dll"), 7, 9);
        t.record(0x2000_0000, 0x6000, &wide("b.dll"), 8, 10);
        assert_eq!(
            (t.elements[0].sequence, t.elements[0].base_address, t.elements[0].size_of_image),
            (1, 0x1000_0000, 0x5000)
        );
        assert_eq!((t.elements[1].sequence, t.elements[1].time_date_stamp, t.elements[1].check_sum), (2, 8, 10));
        let mut name = wide("a.dll");
        name.push(0);
        assert_eq!(&t.elements[0].image_name[..6], &name[..]);
        assert_eq!(t.elements[2].sequence, 0, "unused elements stay empty");
    }

    #[test]
    fn the_65th_event_replaces_the_first_and_names_are_cut_to_31_characters() {
        let mut t = Trace::new();
        for i in 0..65u64 {
            t.record(i + 1, 1, &wide("m.dll"), 0, 0);
        }
        assert_eq!((t.elements[0].sequence, t.elements[0].base_address), (65, 65));
        assert_eq!(t.elements[1].sequence, 2);
        let long = wide(&"x".repeat(40));
        t.record(1, 1, &long, 0, 0);
        let e = t.elements[1];
        assert_eq!(e.image_name[30], u16::from(b'x'));
        assert_eq!(e.image_name[31], 0);
    }
}
