//! `__CxxFrameHandler4`'s compressed tables ("FH4", Visual C++ 2019 and later on x64), as the
//! spec sheet `specs/windows/vcruntime140_1.dll/__CxxFrameHandler4.yaml` (`data_layouts`,
//! `rules`) describes them, and the decisions for one frame that has them.
//!
//! The tables are decoded into the logical structures of the FH3 tables (an unwind map whose
//! entries name the next state, try blocks with their clauses, an IP-to-state map), and the
//! FH3 rules decide: clause matching and the catch object are [`crate::throw`]'s. What differs
//! from FH3:
//!
//! - a catch funclet with objects or try blocks of its own has its *own* table (header bit
//!   0x01) with its own state numbering and the `ParentFrameOffset` of the slot holding the
//!   function's establisher frame; the function's table never covers a funclet's code, so a
//!   frame is always "the function" of the table it names;
//! - the unwind actions come in three kinds (a destructor of an object in the frame, of an
//!   object whose address is in the frame, a cleanup funclet);
//! - a clause may list the continuation addresses (offsets from the function's start), in
//!   which case the catch funclet returns the index of the one to resume at;
//! - the IP-to-state lookup takes the last entry strictly below the control PC.
//!
//! ```text
//! varint      1..5 bytes, the length in the low bits of the first byte (0, 01, 011, 0111, 1111)
//! FuncInfo4   u8 header, [rva UnwindMap if 0x08] [rva TryBlockMap if 0x10] rva IPToState
//!             [rva SeparatedIPMap if 0x02] [varint ParentFrameOffset if 0x01]
//! UnwindMap4  varint count; per entry varint V (type V&3, NextOffset V>>2) then by type:
//!             0 nothing, 1 rva Action + varint ObjectOffset, 2 rva Action + varint
//!             PointerOffset, 3 rva Action
//! TryBlockMap4  varint count; per entry varint TryLow, TryHigh, CatchHigh, rva HandlerMap
//! HandlerMap4   varint count; per clause u8 header, [varint Adjectives if 0x01]
//!               [rva TypeDescriptor if 0x02] [varint CatchObjectOffset if 0x04] rva funclet,
//!               (header & 0x30) >> 4 varint continuation offsets
//! IPToState4  varint count; per entry varint IPDelta, varint StateIndex (state = index - 1)
//! ```

use crate::consts::ADJ_CXX_ONLY;
use crate::tables::HandlerType;
use crate::throw::{Exception, Match, Thrown, match_clause};
use crate::{Bad, MAX_ENTRIES, Memory, at, read_u8, read_u32};
use alloc::vec::Vec;

/// FuncInfo4 header bits.
pub const FH4_IS_CATCH: u8 = 0x01;
pub const FH4_IS_SEPARATED: u8 = 0x02;
pub const FH4_BBT: u8 = 0x04;
pub const FH4_UNWIND_MAP: u8 = 0x08;
pub const FH4_TRY_BLOCK_MAP: u8 = 0x10;
pub const FH4_EHS: u8 = 0x20;
pub const FH4_NOEXCEPT: u8 = 0x40;

/// HandlerMap4 clause header bits.
pub const H4_ADJECTIVES: u8 = 0x01;
pub const H4_TYPE: u8 = 0x02;
pub const H4_CATCH_OBJECT: u8 = 0x04;
pub const H4_CONTINUATIONS: u8 = 0x30;

/// A reader of varints and fixed fields at a moving address.
struct Cursor<'a> {
    mem: &'a dyn Memory,
    addr: u64,
}

impl Cursor<'_> {
    fn u8(&mut self) -> Result<u8, Bad> {
        let b = read_u8(self.mem, self.addr)?;
        self.addr = self.addr.checked_add(1).ok_or(Bad::Overflow)?;
        Ok(b)
    }

    fn u32(&mut self) -> Result<u32, Bad> {
        let v = read_u32(self.mem, self.addr)?;
        self.addr = self.addr.checked_add(4).ok_or(Bad::Overflow)?;
        Ok(v)
    }

    /// The variable-length unsigned integer of the sheet's `varint`.
    fn varint(&mut self) -> Result<u32, Bad> {
        let b0 = self.u8()?;
        let len = match b0 {
            _ if b0 & 0x01 == 0 => 1,
            _ if b0 & 0x03 == 0x01 => 2,
            _ if b0 & 0x07 == 0x03 => 3,
            _ if b0 & 0x0f == 0x07 => 4,
            _ => return self.u32(),
        };
        let mut v = u32::from(b0);
        for i in 1..len {
            v |= u32::from(self.u8()?) << (8 * i);
        }
        Ok(v >> len)
    }

    /// A varint count, refused above [`MAX_ENTRIES`].
    fn count(&mut self) -> Result<u32, Bad> {
        let n = self.varint()?;
        if n > MAX_ENTRIES { Err(Bad::Count(i64::from(n))) } else { Ok(n) }
    }
}

/// Decode one varint from `bytes` (for tests and tools): the value and its length.
pub fn decode_varint(bytes: &[u8]) -> Result<(u32, usize), Bad> {
    let mem = crate::SliceMemory { base: 0x1000, bytes };
    let mut c = Cursor { mem: &mem, addr: 0x1000 };
    let v = c.varint()?;
    Ok((v, (c.addr - 0x1000) as usize))
}

/// What leaving a state does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action4 {
    /// Nothing (a try block's own state, a placeholder).
    None,
    /// `Action(RCX = establisher frame + offset)`: the destructor of an object in the frame.
    Destroy { function: u32, offset: u32 },
    /// `Action(RCX = *(establisher frame + offset))`: the destructor of an object whose address
    /// the frame holds.
    DestroyIndirect { function: u32, offset: u32 },
    /// `Action(RDX = establisher frame)`: a cleanup funclet.
    Funclet { function: u32 },
}

/// One UnwindMap4 entry: its action and the state it goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unwind4 {
    pub to_state: i32,
    pub action: Action4,
}

/// One clause: the FH3 clause fields [`match_clause`] and [`crate::throw::catch_object`] read
/// (`parent_frame_offset` 0: FH4 keeps it in the funclet's own table), and the continuation
/// offsets from the function's start, if listed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Handler4 {
    pub header: u8,
    pub clause: HandlerType,
    pub continuations: [u32; 2],
    pub num_continuations: u8,
}

impl Handler4 {
    /// Where the function goes on after the catch block, from what the funclet returned: with
    /// listed continuations the return value is an index into them, else it is the address.
    pub fn continuation(&self, function_start: u64, returned: u64) -> u64 {
        if self.num_continuations > 0 && returned < u64::from(self.num_continuations) {
            function_start.wrapping_add(u64::from(self.continuations[returned as usize]))
        } else {
            returned
        }
    }
}

/// One TryBlockMap4 entry and its clauses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Try4 {
    pub try_low: i32,
    pub try_high: i32,
    pub catch_high: i32,
    pub handlers: Vec<Handler4>,
}

impl Try4 {
    pub fn covers(&self, state: i32) -> bool {
        self.try_low <= state && state <= self.try_high
    }
}

/// A function's (or catch funclet's) decoded FH4 tables.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Func4 {
    pub header: u8,
    pub unwind: Vec<Unwind4>,
    pub tries: Vec<Try4>,
    /// IP-to-state entries: (rva from the image base, state), ascending.
    pub ip_map: Vec<(u32, i32)>,
    /// For a catch funclet's table: where its frame holds the function's establisher frame.
    pub parent_frame_offset: Option<u32>,
}

fn state_of(i: usize) -> Result<i32, Bad> {
    i32::try_from(i).map_err(|_| Bad::Count(i as i64))
}

impl Func4 {
    /// Decode the FuncInfo4 at `image_base + rva` of the function (or funclet) whose
    /// `RUNTIME_FUNCTION.BeginAddress` is `begin_rva`.
    pub fn read(mem: &dyn Memory, image_base: u64, rva: u32, begin_rva: u32) -> Result<Func4, Bad> {
        let mut c = Cursor { mem, addr: at(image_base, rva, 0)? };
        let header = c.u8()?;
        if header & FH4_IS_SEPARATED != 0 {
            // Hot/cold separated code: not specified (the sheet's notes).
            return Err(Bad::Magic(u32::from(header)));
        }
        let unwind_rva = if header & FH4_UNWIND_MAP != 0 { c.u32()? } else { 0 };
        let try_rva = if header & FH4_TRY_BLOCK_MAP != 0 { c.u32()? } else { 0 };
        let ip_rva = c.u32()?;
        let parent_frame_offset = if header & FH4_IS_CATCH != 0 { Some(c.varint()?) } else { None };
        let unwind = if unwind_rva != 0 { read_unwind_map(mem, at(image_base, unwind_rva, 0)?)? } else { Vec::new() };
        let tries = if try_rva != 0 { read_try_map(mem, image_base, at(image_base, try_rva, 0)?)? } else { Vec::new() };
        let ip_map = if ip_rva != 0 { read_ip_map(mem, at(image_base, ip_rva, 0)?, begin_rva)? } else { Vec::new() };
        let f = Func4 { header, unwind, tries, ip_map, parent_frame_offset };
        f.check()?;
        Ok(f)
    }

    /// Every state the tables name is one of the unwind map's (or -1).
    fn check(&self) -> Result<(), Bad> {
        let n = state_of(self.unwind.len())?;
        let ok = |s: i32| (-1..n).contains(&s);
        for u in &self.unwind {
            if !ok(u.to_state) {
                return Err(Bad::State(u.to_state));
            }
        }
        for t in &self.tries {
            for s in [t.try_low, t.try_high, t.catch_high] {
                if !ok(s) {
                    return Err(Bad::State(s));
                }
            }
        }
        for &(_, s) in &self.ip_map {
            if !ok(s) {
                return Err(Bad::State(s));
            }
        }
        Ok(())
    }

    /// Header bit 0x20: non-C++ exceptions are not this frame's business.
    pub fn synchronous_only(&self) -> bool {
        self.header & FH4_EHS != 0
    }

    /// Header bit 0x40: the function is `noexcept`.
    pub fn is_noexcept(&self) -> bool {
        self.header & FH4_NOEXCEPT != 0
    }

    /// The state for a frame's control PC (an rva): the last entry strictly below it, -1
    /// before the first (a call that never returns has an entry at its own return address).
    pub fn state_at(&self, control_rva: u32) -> i32 {
        let n = self.ip_map.partition_point(|&(ip, _)| ip < control_rva);
        if n == 0 { -1 } else { self.ip_map[n - 1].1 }
    }

    /// The state of the address an unwind goes on at (`longjmp`'s target): the last entry at or
    /// below it.
    pub fn state_at_target(&self, target_rva: u32) -> i32 {
        self.state_at(target_rva.saturating_add(1))
    }

    fn entry(&self, state: i32) -> Result<&Unwind4, Bad> {
        usize::try_from(state).ok().and_then(|i| self.unwind.get(i)).ok_or(Bad::State(state))
    }

    /// The frame's state while a catch block of try block `t` runs: the state around the try
    /// statement (the one its first state goes to).
    pub fn state_during_catch(&self, t: &Try4) -> i32 {
        self.entry(t.try_low).map(|e| e.to_state).unwrap_or(t.try_low.saturating_sub(1))
    }

    /// The actions that take the frame from state `from` down to `to`, in order.
    pub fn unwind_actions(&self, from: i32, to: i32) -> Result<Vec<Action4>, Bad> {
        let mut out = Vec::new();
        let mut s = from;
        let mut steps = 0usize;
        while s > to && s >= 0 {
            let e = self.entry(s)?;
            if e.action != Action4::None {
                out.push(e.action);
            }
            s = e.to_state;
            steps += 1;
            if steps > self.unwind.len() {
                return Err(Bad::Loop);
            }
        }
        Ok(out)
    }

    /// The search's decision for this frame at `state`, for `exc` (a rethrow already completed
    /// if it could be); `image_base` is the base of the image holding the tables.
    pub fn search(&self, mem: &dyn Memory, image_base: u64, state: i32, exc: &Exception) -> Result<Search4, Bad> {
        let covering = || self.tries.iter().enumerate().filter(move |(_, t)| t.covers(state));
        if !exc.is_cxx() {
            if self.synchronous_only() {
                return Ok(Search4::Continue);
            }
            for (i, t) in covering() {
                for (j, h) in t.handlers.iter().enumerate() {
                    if h.clause.type_descriptor == 0 && h.clause.adjectives & ADJ_CXX_ONLY == 0 {
                        return Ok(Search4::Catch { try_index: i, handler_index: j, matched: Match::CatchAll });
                    }
                }
            }
            return Ok(Search4::Continue);
        }
        if exc.is_rethrow() {
            // A rethrow while nothing is being handled matches nothing.
            return Ok(Search4::Continue);
        }
        let thrown = Thrown::read(mem, exc)?;
        for (i, t) in covering() {
            for (j, h) in t.handlers.iter().enumerate() {
                if let Some(m) = match_clause(mem, image_base, &h.clause, exc, &thrown)? {
                    return Ok(Search4::Catch { try_index: i, handler_index: j, matched: m });
                }
            }
        }
        if self.is_noexcept() {
            return Ok(Search4::Terminate);
        }
        Ok(Search4::Continue)
    }
}

/// What the search does with an FH4 frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Search4 {
    Continue,
    /// `terminate()` now: a `noexcept` function no clause of which takes it.
    Terminate,
    /// Clause `handler_index` of try block `try_index` takes it.
    Catch {
        try_index: usize,
        handler_index: usize,
        matched: Match,
    },
}

fn read_unwind_map(mem: &dyn Memory, start: u64) -> Result<Vec<Unwind4>, Bad> {
    let mut c = Cursor { mem, addr: start };
    let n = c.count()?;
    // (offset of the entry from the table's start, NextOffset, action)
    let mut raw: Vec<(u64, u32, Action4)> = Vec::new();
    for _ in 0..n {
        let off = c.addr - start;
        let v = c.varint()?;
        let action = match v & 3 {
            0 => Action4::None,
            1 => {
                let function = c.u32()?;
                Action4::Destroy { function, offset: c.varint()? }
            }
            2 => {
                let function = c.u32()?;
                Action4::DestroyIndirect { function, offset: c.varint()? }
            }
            _ => Action4::Funclet { function: c.u32()? },
        };
        raw.push((off, v >> 2, action));
    }
    let mut out = Vec::with_capacity(raw.len());
    for &(off, next, action) in &raw {
        let target = off.checked_sub(u64::from(next)).ok_or(Bad::Overflow)?;
        let to_state = if target == 0 {
            -1
        } else {
            let i = raw.iter().position(|r| r.0 == target).ok_or(Bad::Index(next))?;
            state_of(i)?
        };
        out.push(Unwind4 { to_state, action });
    }
    Ok(out)
}

fn read_try_map(mem: &dyn Memory, image_base: u64, start: u64) -> Result<Vec<Try4>, Bad> {
    let mut c = Cursor { mem, addr: start };
    let n = c.count()?;
    let mut out = Vec::new();
    for _ in 0..n {
        let try_low = c.varint()? as i32;
        let try_high = c.varint()? as i32;
        let catch_high = c.varint()? as i32;
        let map = c.u32()?;
        let handlers = read_handler_map(mem, at(image_base, map, 0)?)?;
        out.push(Try4 { try_low, try_high, catch_high, handlers });
    }
    Ok(out)
}

fn read_handler_map(mem: &dyn Memory, start: u64) -> Result<Vec<Handler4>, Bad> {
    let mut c = Cursor { mem, addr: start };
    let n = c.count()?;
    let mut out = Vec::new();
    for _ in 0..n {
        let header = c.u8()?;
        let adjectives = if header & H4_ADJECTIVES != 0 { c.varint()? } else { 0 };
        let type_descriptor = if header & H4_TYPE != 0 { c.u32()? } else { 0 };
        let catch_obj_offset = if header & H4_CATCH_OBJECT != 0 { c.varint()? as i32 } else { 0 };
        let handler = c.u32()?;
        let num_continuations = (header & H4_CONTINUATIONS) >> 4;
        let mut continuations = [0u32; 2];
        for slot in continuations.iter_mut().take(usize::from(num_continuations.min(2))) {
            *slot = c.varint()?;
        }
        let num_continuations = num_continuations.min(2);
        let clause = HandlerType { adjectives, type_descriptor, catch_obj_offset, handler, parent_frame_offset: 0 };
        out.push(Handler4 { header, clause, continuations, num_continuations });
    }
    Ok(out)
}

fn read_ip_map(mem: &dyn Memory, start: u64, begin_rva: u32) -> Result<Vec<(u32, i32)>, Bad> {
    let mut c = Cursor { mem, addr: start };
    let n = c.count()?;
    let mut out = Vec::new();
    let mut ip = begin_rva;
    for _ in 0..n {
        ip = ip.checked_add(c.varint()?).ok_or(Bad::Overflow)?;
        let state = i64::from(c.varint()?) - 1;
        out.push((ip, i32::try_from(state).map_err(|_| Bad::Count(state))?));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SliceMemory;
    use crate::consts::*;
    use alloc::vec;

    const BASE: u64 = 0x1_4000_0000;

    #[test]
    fn varints_decode_as_the_sheet_observed() {
        assert_eq!(decode_varint(&[0x08]).unwrap(), (4, 1));
        assert_eq!(decode_varint(&[0x25, 0x11]).unwrap(), (1097, 2));
        assert_eq!(decode_varint(&[0x0b, 0xa2, 0x03]).unwrap(), (29761, 3));
        assert_eq!(decode_varint(&[0x5b, 0x73, 0x55]).unwrap(), (700_011, 3));
        assert_eq!(decode_varint(&[0x83, 0x6b, 0x18]).unwrap(), (200_048, 3));
        assert_eq!(decode_varint(&[0x91, 0x02]).unwrap(), (164, 2));
        // The 4- and 5-byte forms follow the pattern.
        assert_eq!(decode_varint(&[0x07 | (5 << 4), 0, 0, 0]).unwrap(), (5, 4));
        assert_eq!(decode_varint(&[0x0f, 0x78, 0x56, 0x34, 0x12]).unwrap(), (0x1234_5678, 5));
        assert!(decode_varint(&[0x01]).is_err(), "a truncated varint is an error");
    }

    /// An image: bytes at BASE, tables appended.
    struct Img(Vec<u8>);

    impl Img {
        fn new() -> Img {
            Img(vec![0; 0x200])
        }
        fn put(&mut self, b: &[u8]) -> u32 {
            let r = self.0.len() as u32;
            self.0.extend_from_slice(b);
            r
        }
        fn mem(&self) -> SliceMemory<'_> {
            SliceMemory { base: BASE, bytes: &self.0 }
        }
    }

    fn le(v: u32) -> [u8; 4] {
        v.to_le_bytes()
    }

    /// The sheet's worked example: `int f1() { try { thrower(); } catch (int e) { return e; } }`.
    #[test]
    fn worked_example_decodes() {
        let mut img = Img::new();
        let td = img.put(b"\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0.H\0");
        let unwind = img.put(&[0x04, 0x08, 0x10]);
        let mut hm = vec![0x02, 0x16];
        hm.extend(le(td));
        hm.push(0x40);
        hm.extend(le(0x1500));
        hm.push(0x1c);
        let hm = img.put(&hm);
        let mut tm = vec![0x02, 0x00, 0x00, 0x02];
        tm.extend(le(hm));
        let tm = img.put(&tm);
        let ip = img.put(&[0x02, 0x08, 0x02]);
        let mut fi = vec![0x38];
        fi.extend(le(unwind));
        fi.extend(le(tm));
        fi.extend(le(ip));
        let fi = img.put(&fi);
        let m = img.mem();
        let f = Func4::read(&m, BASE, fi, 0x1000).unwrap();
        assert!(f.synchronous_only());
        assert!(!f.is_noexcept());
        assert_eq!(f.parent_frame_offset, None);
        assert_eq!(
            f.unwind,
            vec![Unwind4 { to_state: -1, action: Action4::None }, Unwind4 { to_state: -1, action: Action4::None }]
        );
        assert_eq!(f.tries.len(), 1);
        let t = &f.tries[0];
        assert_eq!((t.try_low, t.try_high, t.catch_high), (0, 0, 1));
        let h = t.handlers[0];
        assert_eq!(h.clause.type_descriptor, td);
        assert_eq!(h.clause.catch_obj_offset, 32);
        assert_eq!(h.clause.handler, 0x1500);
        assert_eq!(h.clause.adjectives, 0);
        assert_eq!((h.num_continuations, h.continuations[0]), (1, 14));
        assert_eq!(h.continuation(0x1_4000_1000, 0), 0x1_4000_1000 + 14);
        assert_eq!(f.ip_map, vec![(0x1004, 0)]);
        // The call at start + 4 returns to a later address: state 0; at start + 4 itself, -1.
        assert_eq!(f.state_at(0x1009), 0);
        assert_eq!(f.state_at(0x1004), -1);
        assert_eq!(f.state_at(0x1000), -1);
        assert_eq!(f.state_at_target(0x1004), 0);
        assert_eq!(f.state_during_catch(t), -1);

        // An int thrown is caught by the clause.
        let ct = img.0.len() as u32;
        let mut b = Vec::new();
        for w in [CT_SIMPLE, td, 0, u32::MAX, 0, 4, 0] {
            b.extend(le(w));
        }
        img.put(&b);
        let mut cta = le(1).to_vec();
        cta.extend(le(ct));
        let cta = img.put(&cta);
        let mut ti = Vec::new();
        for w in [0, 0, 0, cta] {
            ti.extend(le(w));
        }
        let ti = img.put(&ti);
        let obj = img.put(&le(7));
        let m = img.mem();
        let exc = Exception::new(
            CXX_EXCEPTION,
            0x81,
            &[u64::from(MAGIC_V1), BASE + u64::from(obj), BASE + u64::from(ti), BASE],
        );
        let f = Func4::read(&m, BASE, fi, 0x1000).unwrap();
        assert!(matches!(f.search(&m, BASE, 0, &exc).unwrap(), Search4::Catch { try_index: 0, handler_index: 0, .. }));
        assert_eq!(f.search(&m, BASE, -1, &exc).unwrap(), Search4::Continue);
        // A non-C++ exception is ignored by an /EHsc frame.
        let av = Exception::new(0xC000_0005, 0, &[0, 0]);
        assert_eq!(f.search(&m, BASE, 0, &av).unwrap(), Search4::Continue);
    }

    /// The sheet's unwind map example (three objects in a row) and its ip2state example.
    #[test]
    fn unwind_map_follows_next_offsets() {
        let mut img = Img::new();
        let mut um = vec![0x04, 0x0a];
        um.extend(le(0x3000));
        um.extend([0x70, 0x32]);
        um.extend(le(0x3000));
        um.push(0x60);
        let um = img.put(&um);
        let ip = img.put(&[0x04, 0x08, 0x02, 0x0c, 0x04]);
        let mut fi = vec![0x28];
        fi.extend(le(um));
        fi.extend(le(ip));
        let fi = img.put(&fi);
        let m = img.mem();
        let f = Func4::read(&m, BASE, fi, 0x2000).unwrap();
        assert_eq!(
            f.unwind,
            vec![
                Unwind4 { to_state: -1, action: Action4::Destroy { function: 0x3000, offset: 56 } },
                Unwind4 { to_state: 0, action: Action4::Destroy { function: 0x3000, offset: 48 } },
            ]
        );
        assert_eq!(f.ip_map, vec![(0x2004, 0), (0x200a, 1)]);
        assert_eq!(
            f.unwind_actions(1, -1).unwrap(),
            vec![Action4::Destroy { function: 0x3000, offset: 48 }, Action4::Destroy { function: 0x3000, offset: 56 },]
        );
        assert_eq!(f.unwind_actions(1, 0).unwrap().len(), 1);
        assert!(f.unwind_actions(-1, -1).unwrap().is_empty());
        assert!(f.unwind_actions(5, -1).is_err());
    }

    #[test]
    fn catch_funclet_tables_and_other_actions() {
        let mut img = Img::new();
        // Count 3: type 2 (indirect), type 3 (funclet), type 0.
        let mut um = vec![0x06];
        um.push(((1 << 2) | 2) << 1); // offset 1, next 1 -> none
        um.extend(le(0x4000));
        um.push(0x10 << 1);
        let e1 = um.len() as u8;
        um.push((((e1 - 1) << 2) | 3) << 1); // next -> entry 0
        um.extend(le(0x4100));
        let e2 = um.len() as u8;
        um.push(((e2 - e1) << 2) << 1); // type 0, next -> entry 1
        let um = img.put(&um);
        let ip = img.put(&[0x00]);
        let mut fi = vec![0x29];
        fi.extend(le(um));
        fi.extend(le(ip));
        fi.push(56 << 1);
        let fi = img.put(&fi);
        let m = img.mem();
        let f = Func4::read(&m, BASE, fi, 0x2000).unwrap();
        assert_eq!(f.parent_frame_offset, Some(56));
        assert_eq!(
            f.unwind_actions(2, -1).unwrap(),
            vec![Action4::Funclet { function: 0x4100 }, Action4::DestroyIndirect { function: 0x4000, offset: 16 },]
        );
    }

    #[test]
    fn noexcept_frames_terminate_and_bad_tables_are_errors() {
        let mut img = Img::new();
        let ip = img.put(&[0x00]);
        let mut fi = vec![0x60];
        fi.extend(le(ip));
        let fi = img.put(&fi);
        // A throw of an int with no clause anywhere.
        let td = img.put(b"\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0.H\0");
        let mut b = Vec::new();
        for w in [CT_SIMPLE, td, 0, u32::MAX, 0, 4, 0] {
            b.extend(le(w));
        }
        let ct = img.put(&b);
        let mut cta = le(1).to_vec();
        cta.extend(le(ct));
        let cta = img.put(&cta);
        let mut ti = Vec::new();
        for w in [0, 0, 0, cta] {
            ti.extend(le(w));
        }
        let ti = img.put(&ti);
        // A table whose unwind entry points at no entry, and a separated one.
        let bad_um = img.put(&[0x02, (5 << 2) << 1]);
        let mut bad = vec![0x08];
        bad.extend(le(bad_um));
        bad.extend(le(ip));
        let bad = img.put(&bad);
        let mut sep = vec![0x02];
        sep.extend(le(ip));
        sep.extend(le(ip));
        let sep = img.put(&sep);
        let m = img.mem();
        let f = Func4::read(&m, BASE, fi, 0x1000).unwrap();
        assert!(f.is_noexcept());
        let exc = Exception::new(CXX_EXCEPTION, 0x81, &[u64::from(MAGIC_V1), BASE + 0x10, BASE + u64::from(ti), BASE]);
        assert_eq!(f.search(&m, BASE, -1, &exc).unwrap(), Search4::Terminate);
        assert!(Func4::read(&m, BASE, bad, 0x1000).is_err());
        assert!(Func4::read(&m, BASE, sep, 0x1000).is_err());
        assert!(Func4::read(&m, BASE, 0xFFFF_0000, 0x1000).is_err());
    }

    /// Nested tries, as the sheet's observed order: inner (3,4,5), outer (1,5,6), sibling (7,7,8).
    #[test]
    fn inner_try_blocks_are_found_first() {
        let mut img = Img::new();
        let td = img.put(b"\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0.H\0");
        let mut hm = vec![0x02, 0x02];
        hm.extend(le(td));
        hm.extend(le(0x5000));
        let hm1 = img.put(&hm);
        let mut hm = vec![0x02, 0x11, ADJ_CXX_ONLY as u8 * 2];
        hm.extend(le(0x5100));
        let hm2 = img.put(&hm);
        let mut tm = vec![0x06];
        for (lo, hi, ch, h) in [(3u8, 4u8, 5u8, hm1), (1, 5, 6, hm2), (7, 7, 8, hm2)] {
            tm.extend([lo * 2, hi * 2, ch * 2]);
            tm.extend(le(h));
        }
        let tm = img.put(&tm);
        let um = img.put(&[0x12, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08]);
        let ip = img.put(&[0x00]);
        let mut fi = vec![0x18];
        fi.extend(le(um));
        fi.extend(le(tm));
        fi.extend(le(ip));
        let fi = img.put(&fi);
        let m = img.mem();
        let f = Func4::read(&m, BASE, fi, 0x1000).unwrap();
        assert_eq!(f.unwind.len(), 9);
        assert!(!f.synchronous_only());
        // An access violation at state 4: the inner clause is typed, the outer catch(...) is
        // C++ only: nothing takes it.
        let av = Exception::new(0xC000_0005, 0, &[0, 0]);
        assert_eq!(f.search(&m, BASE, 4, &av).unwrap(), Search4::Continue);
    }
}
