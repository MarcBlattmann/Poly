//! Host tests on synthetic tables: an image built in a byte buffer with
//! the layouts of the spec sheets, laid out the way the compiler lays out
//! a function's tables (the shapes of the visible tests' functions).

extern crate std;

use crate::catches::Catches;
use crate::consts::*;
use crate::frame::*;
use crate::tables::*;
use crate::throw::*;
use crate::{Bad, Memory, SliceMemory};
use alloc::vec;
use alloc::vec::Vec;

const BASE: u64 = 0x1_4000_0000;

/// An image under construction.
struct Image {
    bytes: Vec<u8>,
}

impl Image {
    fn new() -> Image {
        Image { bytes: vec![0; 0x100] }
    }

    fn alloc(&mut self, data: &[u8]) -> u32 {
        while !self.bytes.len().is_multiple_of(8) {
            self.bytes.push(0);
        }
        let rva = self.bytes.len() as u32;
        self.bytes.extend_from_slice(data);
        rva
    }

    fn words(&mut self, w: &[u32]) -> u32 {
        let b: Vec<u8> = w.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.alloc(&b)
    }

    fn type_descriptor(&mut self, name: &str) -> u32 {
        let mut b = vec![0u8; 16];
        b.extend_from_slice(name.as_bytes());
        b.push(0);
        self.alloc(&b)
    }

    fn put_u64(&mut self, rva: u32, v: u64) {
        self.bytes[rva as usize..rva as usize + 8].copy_from_slice(&v.to_le_bytes());
    }

    fn put_i32(&mut self, rva: u32, v: i32) {
        self.bytes[rva as usize..rva as usize + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn mem(&self) -> SliceMemory<'_> {
        SliceMemory { base: BASE, bytes: &self.bytes }
    }

    fn addr(rva: u32) -> u64 {
        BASE + u64::from(rva)
    }
}

/// A clause: adjectives, type descriptor rva, catch object offset, funclet
/// rva.
fn clause(adjectives: u32, td: u32, obj: i32, funclet: u32) -> HandlerType {
    HandlerType { adjectives, type_descriptor: td, catch_obj_offset: obj, handler: funclet, parent_frame_offset: 56 }
}

/// A function's tables.
#[derive(Default)]
struct Spec {
    magic: u32,
    eh_flags: u32,
    unwind: Vec<(i32, u32)>,
    tries: Vec<(i32, i32, i32, Vec<HandlerType>)>,
    ip: Vec<(u32, i32)>,
}

fn build(img: &mut Image, s: &Spec) -> u32 {
    let unwind: Vec<u32> = s.unwind.iter().flat_map(|&(t, a)| [t as u32, a]).collect();
    let unwind_rva = if unwind.is_empty() { 0 } else { img.words(&unwind) };
    let mut tries = Vec::new();
    for (lo, hi, ch, handlers) in &s.tries {
        let hw: Vec<u32> = handlers
            .iter()
            .flat_map(|h| {
                [h.adjectives, h.type_descriptor, h.catch_obj_offset as u32, h.handler, h.parent_frame_offset]
            })
            .collect();
        let hr = img.words(&hw);
        tries.extend_from_slice(&[*lo as u32, *hi as u32, *ch as u32, handlers.len() as u32, hr]);
    }
    let try_rva = if tries.is_empty() { 0 } else { img.words(&tries) };
    let ip: Vec<u32> = s.ip.iter().flat_map(|&(a, st)| [a, st as u32]).collect();
    let ip_rva = if ip.is_empty() { 0 } else { img.words(&ip) };
    let magic = if s.magic == 0 { MAGIC_V3 } else { s.magic };
    img.words(&[
        magic,
        s.unwind.len() as u32,
        unwind_rva,
        s.tries.len() as u32,
        try_rva,
        s.ip.len() as u32,
        ip_rva,
        64,
        0,
        s.eh_flags,
    ])
}

/// A catchable type: properties, descriptor, mdisp, pdisp, vdisp, size,
/// copy function.
type Ct = (u32, u32, i32, i32, i32, i32, u32);

fn throw_info(img: &mut Image, attributes: u32, destructor: u32, types: &[Ct]) -> u32 {
    let mut rvas = Vec::new();
    for &(p, td, m, pd, vd, sz, cp) in types {
        rvas.push(img.words(&[p, td, m as u32, pd as u32, vd as u32, sz as u32, cp]));
    }
    let mut cta = vec![types.len() as u32];
    cta.extend(rvas);
    let cta = img.words(&cta);
    img.words(&[attributes, destructor, 0, cta])
}

fn cxx(object: u64, ti: u32) -> Exception {
    Exception::new(CXX_EXCEPTION, 0x81, &[u64::from(MAGIC_V1), object, Image::addr(ti), BASE])
}

fn func<'a>(mem: &'a SliceMemory<'a>, rva: u32) -> Func<'a> {
    Func::read(mem, BASE, rva).unwrap()
}

// ----- tables ------------------------------------------------------------------------

#[test]
fn func_info_reads_only_its_magic_fields() {
    let mut img = Image::new();
    // Every magic number with 7 and 5 in the words after the oldest layout.
    for (magic, es, flags) in [(MAGIC_V1, 0, 0), (MAGIC_V2, 7, 0), (MAGIC_V3, 7, 5)] {
        let rva = img.words(&[magic | 0xE000_0000, 3, 0x10, 2, 0x20, 4, 0x30, 64, 7, 5]);
        let fi = FuncInfo::read(&img.mem(), Image::addr(rva)).unwrap();
        assert_eq!(fi.magic, magic, "upper 3 bits ignored");
        assert_eq!((fi.max_state, fi.unwind_map, fi.num_try_blocks, fi.try_block_map), (3, 0x10, 2, 0x20));
        assert_eq!((fi.ip_map_entries, fi.ip_to_state_map, fi.unwind_help), (4, 0x30, 64));
        assert_eq!((fi.es_type_list, fi.eh_flags), (es, flags));
        assert_eq!(fi.synchronous_only(), flags & 1 != 0);
        assert_eq!(fi.is_noexcept(), flags & 4 != 0);
    }
    // The oldest layout at the very end of readable memory: nothing after it
    // is read.
    let mut b = vec![0u8; 0x100];
    let w = [MAGIC_V1, 0, 0, 0, 0, 0, 0, 0];
    for (i, x) in w.iter().enumerate() {
        b[0xE0 + 4 * i..0xE4 + 4 * i].copy_from_slice(&x.to_le_bytes());
    }
    let m = SliceMemory { base: BASE, bytes: &b };
    assert!(FuncInfo::read(&m, BASE + 0xE0).is_ok());
    // V2 there needs one more word.
    b[0xE0..0xE4].copy_from_slice(&MAGIC_V2.to_le_bytes());
    let m = SliceMemory { base: BASE, bytes: &b };
    assert!(matches!(FuncInfo::read(&m, BASE + 0xE0), Err(Bad::Unreadable(_))));
}

#[test]
fn bad_magic_is_refused() {
    let mut img = Image::new();
    for magic in [0, 0x1993_051F, 0x1993_0523, 0x0123_4567] {
        let rva = img.words(&[magic, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(FuncInfo::read(&img.mem(), Image::addr(rva)), Err(Bad::Magic(magic)));
    }
}

#[test]
fn ip_to_state_is_the_last_entry_at_or_below() {
    let mut img = Image::new();
    let rva = build(
        &mut img,
        &Spec {
            unwind: vec![(-1, 0), (0, 0)],
            ip: vec![(0x1000, -1), (0x1011, 0), (0x1020, 1), (0x1030, -1)],
            ..Default::default()
        },
    );
    let m = img.mem();
    let f = func(&m, rva);
    for (pc, want) in
        [(0xFFF, -1), (0x1000, -1), (0x1010, -1), (0x1011, 0), (0x101F, 0), (0x1020, 1), (0x1030, -1), (0x9999, -1)]
    {
        assert_eq!(f.state_at(pc).unwrap(), want, "pc {pc:#x}");
    }
    // No entries: -1 everywhere.
    let rva = build(&mut img, &Spec::default());
    let m = img.mem();
    assert_eq!(func(&m, rva).state_at(0x1234).unwrap(), -1);
}

#[test]
fn table_indices_are_checked() {
    let mut img = Image::new();
    let rva = build(&mut img, &Spec { unwind: vec![(-1, 0)], tries: vec![(0, 0, 0, vec![])], ..Default::default() });
    let m = img.mem();
    let f = func(&m, rva);
    assert_eq!(f.unwind_entry(1), Err(Bad::State(1)));
    assert_eq!(f.unwind_entry(-1), Err(Bad::State(-1)));
    assert_eq!(f.try_block(1), Err(Bad::Index(1)));
    let tb = f.try_block(0).unwrap();
    assert_eq!(f.handler(&tb, 0), Err(Bad::Index(0)));
}

// ----- the shapes of the visible tests --------------------------------------------------

/// `try { try { A } catch (X&) [2] } catch (int) [3]` with a try block in a
/// catch block and a two-level nesting, as clang numbers them (the
/// visible test cxx3_nested_try): states and try blocks as in its tables.
fn nested(img: &mut Image) -> (u32, u32, u32) {
    let x = img.type_descriptor(".?AUX@@");
    let int = img.type_descriptor(".H");
    let s = Spec {
        // 0..3: inner/outer try; 12..17: the two-level case.
        unwind: vec![
            (-1, 0),
            (0, 0),
            (0, 0),
            (-1, 0),
            (-1, 0),
            (-1, 0),
            (-1, 0),
            (-1, 0),
            (-1, 0),
            (-1, 0),
            (-1, 0),
            (-1, 0),
            (-1, 0),
            (12, 0),
            (12, 0),
            (14, 0),
            (14, 0),
            (-1, 0),
        ],
        tries: vec![
            (1, 1, 2, vec![clause(8, x, 0, 0x5010)]),
            (0, 2, 3, vec![clause(0, int, 124, 0x5020)]),
            (13, 13, 16, vec![clause(8, x, 96, 0x5030)]),
            (15, 15, 16, vec![clause(8, x, 88, 0x5040)]),
            (12, 16, 17, vec![clause(0, int, 84, 0x5050)]),
        ],
        ip: vec![(0x1000, -1)],
        ..Default::default()
    };
    (build(img, &s), x, int)
}

#[test]
fn frame_kinds_follow_the_state() {
    let mut img = Image::new();
    let (rva, _, _) = nested(&mut img);
    let m = img.mem();
    let f = func(&m, rva);
    assert_eq!(frame_kind(&f, -1, 0x1000).unwrap(), FrameKind::Function);
    assert_eq!(frame_kind(&f, 1, 0x1000).unwrap(), FrameKind::Function);
    assert_eq!(frame_kind(&f, 2, 0x5010).unwrap(), FrameKind::Catch { try_index: 0, handler: Some(0) });
    assert_eq!(frame_kind(&f, 3, 0x5020).unwrap(), FrameKind::Catch { try_index: 1, handler: Some(0) });
    // 16 is in the catch states of try blocks 2 (14..16) and 3 (16..16):
    // the inner one.
    assert_eq!(frame_kind(&f, 16, 0x5040).unwrap(), FrameKind::Catch { try_index: 3, handler: Some(0) });
    assert_eq!(frame_kind(&f, 14, 0x5030).unwrap(), FrameKind::Catch { try_index: 2, handler: Some(0) });
    // A funclet that is no clause of the try block: still its catch block.
    assert_eq!(frame_kind(&f, 14, 0x7777).unwrap(), FrameKind::Catch { try_index: 2, handler: None });
}

#[test]
fn an_unwind_funclet_is_a_cleanup_frame() {
    let mut img = Image::new();
    let rva = build(&mut img, &Spec { unwind: vec![(-1, 0x6000), (0, 0x6100)], ..Default::default() });
    let m = img.mem();
    let f = func(&m, rva);
    assert_eq!(frame_kind(&f, 1, 0x6100).unwrap(), FrameKind::Cleanup);
    assert_eq!(frame_kind(&f, 1, 0x6000).unwrap(), FrameKind::Cleanup);
    assert_eq!(frame_kind(&f, 1, 0x1000).unwrap(), FrameKind::Function);
    assert!(candidates(&f, FrameKind::Cleanup, 1).unwrap().is_empty());
    assert!(unwind_actions(&f, FrameKind::Cleanup, 1, -1).unwrap().is_empty());
}

#[test]
fn candidates_inner_first_and_only_the_frames_own() {
    let mut img = Image::new();
    let (rva, _, _) = nested(&mut img);
    let m = img.mem();
    let f = func(&m, rva);
    // In the inner try body: the inner try block, then the outer one.
    assert_eq!(candidates(&f, FrameKind::Function, 1).unwrap(), vec![0, 1]);
    // In the outer try body only.
    assert_eq!(candidates(&f, FrameKind::Function, 0).unwrap(), vec![1]);
    // The function while the two-level catch blocks run: state 12, the
    // enclosing try block.
    assert_eq!(candidates(&f, FrameKind::Function, 12).unwrap(), vec![4]);
    // A catch funclet's frame at 15 (a try block inside its catch block):
    // that one only, not try block 4, which encloses the whole statement.
    let k = frame_kind(&f, 14, 0x5030).unwrap();
    assert_eq!(candidates(&f, k, 15).unwrap(), vec![3]);
    assert!(candidates(&f, k, 14).unwrap().is_empty());
    // The inner catch funclet at 16: nothing of its own.
    let k = frame_kind(&f, 16, 0x5040).unwrap();
    assert!(candidates(&f, k, 16).unwrap().is_empty());
}

#[test]
fn state_during_catch_is_around_the_try_statement() {
    let mut img = Image::new();
    let (rva, _, _) = nested(&mut img);
    let m = img.mem();
    let f = func(&m, rva);
    assert_eq!(state_during_catch(&f, &f.try_block(0).unwrap()), 0);
    assert_eq!(state_during_catch(&f, &f.try_block(1).unwrap()), -1);
    assert_eq!(state_during_catch(&f, &f.try_block(3).unwrap()), 14);
    assert_eq!(state_during_catch(&f, &f.try_block(2).unwrap()), 12);
    // A try block whose TryLow is outside the map: one below it.
    let bad = TryBlock { try_low: 99, try_high: 99, catch_high: 100, num_catches: 0, handlers: 0 };
    assert_eq!(state_during_catch(&f, &bad), 98);
}

#[test]
fn parent_frame_offset_of_a_catch_frame() {
    let mut img = Image::new();
    let (rva, _, _) = nested(&mut img);
    let m = img.mem();
    let f = func(&m, rva);
    assert_eq!(parent_frame_offset(&f, FrameKind::Function).unwrap(), None);
    let k = frame_kind(&f, 16, 0x5040).unwrap();
    assert_eq!(parent_frame_offset(&f, k).unwrap(), Some(56));
}

// ----- unwinding --------------------------------------------------------------------

#[test]
fn unwind_actions_follow_to_state() {
    let mut img = Image::new();
    // level3 of cxx3_unwind_order: a(0) -> gone(1) left, b(2) after a.
    let rva =
        build(&mut img, &Spec { unwind: vec![(-1, 0x6000), (0, 0x6100), (0, 0x6200), (2, 0)], ..Default::default() });
    let m = img.mem();
    let f = func(&m, rva);
    assert_eq!(unwind_actions(&f, FrameKind::Function, 2, -1).unwrap(), vec![0x6200, 0x6000]);
    assert_eq!(unwind_actions(&f, FrameKind::Function, 3, -1).unwrap(), vec![0x6200, 0x6000]);
    assert_eq!(unwind_actions(&f, FrameKind::Function, 1, -1).unwrap(), vec![0x6100, 0x6000]);
    assert_eq!(unwind_actions(&f, FrameKind::Function, 2, 0).unwrap(), vec![0x6200]);
    assert!(unwind_actions(&f, FrameKind::Function, 0, 0).unwrap().is_empty());
    assert!(unwind_actions(&f, FrameKind::Function, -1, -1).unwrap().is_empty());
    assert_eq!(unwind_actions(&f, FrameKind::Function, 4, -1), Err(Bad::State(4)));
}

#[test]
fn a_catch_frame_unwinds_its_own_states_only() {
    let mut img = Image::new();
    // An object (0), then try(1) { } catch [2: its catch object 3 -> 2].
    let rva = build(
        &mut img,
        &Spec {
            unwind: vec![(-1, 0x6000), (0, 0), (0, 0), (2, 0x6200)],
            tries: vec![(1, 1, 3, vec![clause(0, 0, 0, 0x5000)])],
            ..Default::default()
        },
    );
    let m = img.mem();
    let f = func(&m, rva);
    let k = frame_kind(&f, 3, 0x5000).unwrap();
    assert_eq!(k, FrameKind::Catch { try_index: 0, handler: Some(0) });
    // Its catch object, not the function's object of state 0.
    assert_eq!(unwind_actions(&f, k, 3, -1).unwrap(), vec![0x6200]);
    assert_eq!(unwind_actions(&f, FrameKind::Function, 3, -1).unwrap(), vec![0x6200, 0x6000]);
}

#[test]
fn unwind_loops_end() {
    let mut img = Image::new();
    let rva = build(&mut img, &Spec { unwind: vec![(-1, 0), (2, 0x6000), (1, 0x6100)], ..Default::default() });
    let m = img.mem();
    let f = func(&m, rva);
    assert_eq!(unwind_actions(&f, FrameKind::Function, 2, -1), Err(Bad::Loop));
}

// ----- matching and the search ----------------------------------------------------------

struct Types {
    int: u32,
    long: u32,
    uint: u32,
    base: u32,
    derived: u32,
    intptr: u32,
    voidptr: u32,
}

fn types(img: &mut Image) -> Types {
    Types {
        int: img.type_descriptor(".H"),
        long: img.type_descriptor(".J"),
        uint: img.type_descriptor(".I"),
        base: img.type_descriptor(".?AUBase@@"),
        derived: img.type_descriptor(".?AUDerived@@"),
        intptr: img.type_descriptor(".PEAH"),
        voidptr: img.type_descriptor(".PEAX"),
    }
}

/// One try block covering state 0 with these clauses.
fn one_try(img: &mut Image, clauses: Vec<HandlerType>, eh_flags: u32, magic: u32) -> u32 {
    build(
        img,
        &Spec {
            magic,
            eh_flags,
            unwind: vec![(-1, 0), (-1, 0)],
            tries: vec![(0, 0, 1, clauses)],
            ..Default::default()
        },
    )
}

#[test]
fn first_matching_clause_wins_no_conversions() {
    let mut img = Image::new();
    let t = types(&mut img);
    let rva = one_try(
        &mut img,
        vec![
            clause(0, t.long, 0, 0x5000),
            clause(0, t.uint, 0, 0x5010),
            clause(0, t.int, 0x40, 0x5020),
            clause(0x40, 0, 0, 0x5030),
        ],
        1,
        0,
    );
    let obj = img.words(&[42]);
    let ti = throw_info(&mut img, 0, 0, &[(1, t.int, 0, -1, 0, 4, 0)]);
    let m = img.mem();
    let f = func(&m, rva);
    match search(&f, FrameKind::Function, 0, &cxx(Image::addr(obj), ti)).unwrap() {
        Search::Catch { try_index: 0, handler_index: 2, matched: Match::Typed(ct), .. } => assert_eq!(ct.size, 4),
        other => panic!("{other:?}"),
    }
    // Outside the try block: nothing.
    assert_eq!(search(&f, FrameKind::Function, -1, &cxx(Image::addr(obj), ti)).unwrap(), Search::Continue);
    // A long: the first clause.
    let ti = throw_info(&mut img, 0, 0, &[(1, t.long, 0, -1, 0, 4, 0)]);
    let m = img.mem();
    let f = func(&m, rva);
    assert!(matches!(
        search(&f, FrameKind::Function, 0, &cxx(Image::addr(obj), ti)).unwrap(),
        Search::Catch { handler_index: 0, .. }
    ));
    // Something else: catch(...).
    let ti = throw_info(&mut img, 0, 0, &[(1, t.base, 0, -1, 0, 4, 0)]);
    let m = img.mem();
    let f = func(&m, rva);
    assert!(matches!(
        search(&f, FrameKind::Function, 0, &cxx(Image::addr(obj), ti)).unwrap(),
        Search::Catch { handler_index: 3, matched: Match::CatchAll, .. }
    ));
}

#[test]
fn names_match_across_descriptors_and_array_order() {
    let mut img = Image::new();
    let t = types(&mut img);
    // The thrown side uses its own descriptors (another address, a vftable
    // pointer of garbage) with the same names.
    let their_base = img.type_descriptor(".?AUBase@@");
    img.put_u64(their_base, 0xDEAD_BEEF);
    let their_derived = img.type_descriptor(".?AUDerived@@");
    let rva = one_try(&mut img, vec![clause(8, t.base, 0, 0x5000), clause(8, t.derived, 0, 0x5010)], 1, 0);
    let obj = img.alloc(&[0; 32]);
    let ti =
        throw_info(&mut img, 0, 0, &[(0, their_derived, 0, -1, 0, 24, 0x7000), (0, their_base, 8, -1, 0, 16, 0x7100)]);
    let m = img.mem();
    let f = func(&m, rva);
    // The base clause listed first wins, through the second catchable type.
    match search(&f, FrameKind::Function, 0, &cxx(Image::addr(obj), ti)).unwrap() {
        Search::Catch { handler_index: 0, matched: Match::Typed(ct), .. } => assert_eq!(ct.mdisp, 8),
        other => panic!("{other:?}"),
    }
    assert!(same_name(&m, Image::addr(t.base), Image::addr(their_base)).unwrap());
    assert!(!same_name(&m, Image::addr(t.base), Image::addr(their_derived)).unwrap());
    assert!(!same_name(&m, Image::addr(t.int), Image::addr(t.intptr)).unwrap());
}

#[test]
fn qualifiers_of_the_throw_must_be_in_the_clause() {
    let mut img = Image::new();
    let t = types(&mut img);
    let rva = one_try(&mut img, vec![clause(0, t.intptr, 0, 0x5000), clause(ADJ_CONST, t.intptr, 0, 0x5010)], 1, 0);
    let target = img.words(&[77]);
    let obj = img.alloc(&Image::addr(target).to_le_bytes());
    let ti = throw_info(&mut img, TI_CONST, 0, &[(1, t.intptr, 0, -1, 0, 8, 0), (1, t.voidptr, 0, -1, 0, 8, 0)]);
    let m = img.mem();
    let f = func(&m, rva);
    assert!(matches!(
        search(&f, FrameKind::Function, 0, &cxx(Image::addr(obj), ti)).unwrap(),
        Search::Catch { handler_index: 1, .. }
    ));
    // Volatile is not const.
    let ti = throw_info(&mut img, TI_VOLATILE, 0, &[(1, t.intptr, 0, -1, 0, 8, 0)]);
    let m = img.mem();
    let f = func(&m, rva);
    assert_eq!(search(&f, FrameKind::Function, 0, &cxx(Image::addr(obj), ti)).unwrap(), Search::Continue);
}

#[test]
fn rethrow_without_exception_matches_nothing() {
    let mut img = Image::new();
    let t = types(&mut img);
    let rva = one_try(&mut img, vec![clause(0, t.int, 0, 0x5000), clause(0x40, 0, 0, 0x5010)], 5, 0);
    let m = img.mem();
    let f = func(&m, rva);
    let rethrow = Exception::new(CXX_EXCEPTION, 0x81, &[u64::from(MAGIC_V1), 0, 0, 0]);
    assert!(rethrow.is_rethrow());
    // Not even catch(...), and no terminate in a noexcept frame.
    assert_eq!(search(&f, FrameKind::Function, 0, &rethrow).unwrap(), Search::Continue);
}

#[test]
fn rethrow_records_are_completed_from_the_current_exception() {
    let current = Exception::new(CXX_EXCEPTION, 0x81, &[u64::from(MAGIC_V1), 0x1000, 0x2000, BASE]);
    let mut r = Exception::new(CXX_EXCEPTION, 0x81, &[u64::from(MAGIC_V1), 0, 0, 0]);
    assert!(!r.complete_rethrow(None));
    let other = Exception::new(0xE000_0001, 0, &[]);
    assert!(!r.complete_rethrow(Some(&other)));
    assert!(r.complete_rethrow(Some(&current)));
    assert_eq!(r.params, [u64::from(MAGIC_V1), 0x1000, 0x2000, BASE]);
    assert!(!r.is_rethrow());
    // A record that is not a rethrow is left alone.
    let mut plain = current;
    assert!(!plain.complete_rethrow(Some(&Exception::new(CXX_EXCEPTION, 0x81, &[u64::from(MAGIC_V1), 9, 9, 9]))));
    assert_eq!(plain, current);
}

#[test]
fn non_cxx_exceptions_by_flags_and_adjective() {
    let mut img = Image::new();
    let t = types(&mut img);
    let seh = Exception::new(0xE000_0001, 0x80, &[]);
    assert!(!seh.is_cxx());
    let clauses = || vec![clause(0, t.int, 0, 0x5000), clause(0x40, 0, 0, 0x5010), clause(0, 0, 0, 0x5020)];
    // Synchronous only: not seen.
    let rva = one_try(&mut img, clauses(), EH_SYNCHRONOUS, 0);
    let m = img.mem();
    assert_eq!(search(&func(&m, rva), FrameKind::Function, 0, &seh).unwrap(), Search::Continue);
    // Flags 0: the catch(...) without 0x40, never the typed clause.
    let rva = one_try(&mut img, clauses(), 0, 0);
    let m = img.mem();
    assert!(matches!(
        search(&func(&m, rva), FrameKind::Function, 0, &seh).unwrap(),
        Search::Catch { handler_index: 2, matched: Match::CatchAll, .. }
    ));
    // The older magic numbers have no flags, even with a 1 after the
    // structure.
    for magic in [MAGIC_V1, MAGIC_V2] {
        let rva = one_try(&mut img, clauses(), 1, magic);
        let m = img.mem();
        assert!(matches!(
            search(&func(&m, rva), FrameKind::Function, 0, &seh).unwrap(),
            Search::Catch { handler_index: 2, .. }
        ));
    }
    // With 0x40 only: nothing.
    let rva = one_try(&mut img, vec![clause(0x40, 0, 0, 0x5010)], 0, 0);
    let m = img.mem();
    assert_eq!(search(&func(&m, rva), FrameKind::Function, 0, &seh).unwrap(), Search::Continue);
}

#[test]
fn noexcept_frames_terminate_without_a_matching_clause() {
    let mut img = Image::new();
    let t = types(&mut img);
    let obj = img.words(&[2]);
    let ti = throw_info(&mut img, 0, 0, &[(1, t.int, 0, -1, 0, 4, 0)]);
    // No try block at all.
    let rva = build(&mut img, &Spec { eh_flags: EH_NOEXCEPT | 1, unwind: vec![(-1, 0x6000)], ..Default::default() });
    let m = img.mem();
    assert_eq!(search(&func(&m, rva), FrameKind::Function, 0, &cxx(Image::addr(obj), ti)).unwrap(), Search::Terminate);
    // A matching clause: caught.
    let rva = one_try(&mut img, vec![clause(0, t.int, 0, 0x5000)], 5, 0);
    let m = img.mem();
    assert!(matches!(
        search(&func(&m, rva), FrameKind::Function, 0, &cxx(Image::addr(obj), ti)).unwrap(),
        Search::Catch { .. }
    ));
    // The flags word of the older layouts is not read.
    let rva = build(&mut img, &Spec { magic: MAGIC_V2, eh_flags: 5, unwind: vec![(-1, 0)], ..Default::default() });
    let m = img.mem();
    assert_eq!(search(&func(&m, rva), FrameKind::Function, 0, &cxx(Image::addr(obj), ti)).unwrap(), Search::Continue);
    // Non-C++ exceptions do not terminate.
    let rva = build(&mut img, &Spec { eh_flags: EH_NOEXCEPT, unwind: vec![(-1, 0)], ..Default::default() });
    let m = img.mem();
    assert_eq!(
        search(&func(&m, rva), FrameKind::Function, 0, &Exception::new(0xC000_0005, 0, &[0, 0])).unwrap(),
        Search::Continue
    );
}

#[test]
fn search_in_a_catch_frame_skips_the_enclosing_try() {
    let mut img = Image::new();
    let (rva, _, _) = nested(&mut img);
    let int = img.type_descriptor(".H");
    let obj = img.words(&[7]);
    let ti = throw_info(&mut img, 0, 0, &[(1, int, 0, -1, 0, 4, 0)]);
    let m = img.mem();
    let f = func(&m, rva);
    let e = cxx(Image::addr(obj), ti);
    // The inner catch funclet's frame: not try block 4 (catch(int)).
    let k = frame_kind(&f, 16, 0x5040).unwrap();
    assert_eq!(search(&f, k, 16, &e).unwrap(), Search::Continue);
    // The outer catch funclet's frame at the state around try block 3.
    let k = frame_kind(&f, 14, 0x5030).unwrap();
    assert_eq!(search(&f, k, 14, &e).unwrap(), Search::Continue);
    // The function at the state around try block 2: try block 4 takes it.
    assert!(matches!(search(&f, FrameKind::Function, 12, &e).unwrap(), Search::Catch { try_index: 4, .. }));
}

// ----- addresses and catch objects ------------------------------------------------------

#[test]
fn adjustment_by_mdisp_and_virtual_base() {
    let mut img = Image::new();
    // An object with a vbptr at 8 whose table says the virtual base is 40
    // bytes from the vbptr (entry at vdisp 4).
    let vbtable = img.words(&[0, 40]);
    let obj = img.alloc(&[0; 64]);
    img.put_u64(obj + 8, Image::addr(vbtable));
    let m = img.mem();
    let o = Image::addr(obj);
    let plain = CatchableType { mdisp: 16, pdisp: -1, ..Default::default() };
    assert_eq!(adjust(&m, o, &plain).unwrap(), o + 16);
    let virt = CatchableType { mdisp: 4, pdisp: 8, vdisp: 4, ..Default::default() };
    assert_eq!(adjust(&m, o, &virt).unwrap(), o + 8 + 40 + 4);
    // An unreadable vbptr is an error, not a guess.
    let bad = CatchableType { pdisp: 0x10_0000, ..Default::default() };
    assert!(matches!(adjust(&m, o, &bad), Err(Bad::Unreadable(_))));
}

#[test]
fn catch_objects() {
    let mut img = Image::new();
    let t = types(&mut img);
    let frame = 0x7000_0000u64;
    let obj = img.alloc(&[0; 32]);
    let o = Image::addr(obj);
    let e = cxx(o, 0);
    let m = img.mem();
    let simple_int =
        CatchableType { properties: CT_SIMPLE, type_descriptor: t.int, pdisp: -1, size: 4, ..Default::default() };
    // catch(...) and an unnamed parameter: nothing.
    assert_eq!(catch_object(&m, &clause(0, 0, 0x20, 0), &Match::CatchAll, &e, frame).unwrap(), CatchObject::None);
    assert_eq!(
        catch_object(&m, &clause(0, t.int, 0, 0), &Match::Typed(simple_int), &e, frame).unwrap(),
        CatchObject::None
    );
    // int by value: four bytes.
    assert_eq!(
        catch_object(&m, &clause(0, t.int, 0x7C, 0), &Match::Typed(simple_int), &e, frame).unwrap(),
        CatchObject::Bytes { at: frame + 0x7C, from: o, size: 4 }
    );
    // A reference to a base at offset 8: the adjusted address, no copy.
    let base =
        CatchableType { type_descriptor: t.base, mdisp: 8, pdisp: -1, size: 16, copy: 0x7100, ..Default::default() };
    assert_eq!(
        catch_object(&m, &clause(ADJ_REFERENCE, t.base, 0x40, 0), &Match::Typed(base), &e, frame).unwrap(),
        CatchObject::Value { at: frame + 0x40, value: o + 8 }
    );
    // By value with a copy function; with virtual bases the flag.
    assert_eq!(
        catch_object(&m, &clause(0, t.base, 0x40, 0), &Match::Typed(base), &e, frame).unwrap(),
        CatchObject::Copy { at: frame + 0x40, from: o + 8, function: BASE + 0x7100, virtual_bases: false }
    );
    let vb = CatchableType { properties: CT_VIRTUAL_BASES, ..base };
    assert!(matches!(
        catch_object(&m, &clause(0, t.base, 0x40, 0), &Match::Typed(vb), &e, frame).unwrap(),
        CatchObject::Copy { virtual_bases: true, .. }
    ));
    // Without one: size bytes from the adjusted address.
    let plain = CatchableType { copy: 0, size: 12, ..base };
    assert_eq!(
        catch_object(&m, &clause(0, t.base, 0x40, 0), &Match::Typed(plain), &e, frame).unwrap(),
        CatchObject::Bytes { at: frame + 0x40, from: o + 8, size: 12 }
    );
    // A negative size is refused.
    let neg = CatchableType { size: -1, ..plain };
    assert!(catch_object(&m, &clause(0, t.base, 0x40, 0), &Match::Typed(neg), &e, frame).is_err());
}

#[test]
fn pointers_are_converted_like_addresses() {
    let mut img = Image::new();
    let t = types(&mut img);
    let pointee = img.alloc(&[0; 32]);
    let slot = img.alloc(&Image::addr(pointee).to_le_bytes());
    let null = img.alloc(&[0; 8]);
    let m = img.mem();
    let base_ptr = CatchableType {
        properties: CT_SIMPLE,
        type_descriptor: t.base,
        mdisp: 8,
        pdisp: -1,
        size: 8,
        ..Default::default()
    };
    let h = clause(0, t.base, 0x30, 0);
    assert_eq!(
        catch_object(&m, &h, &Match::Typed(base_ptr), &cxx(Image::addr(slot), 0), 0x7000).unwrap(),
        CatchObject::Value { at: 0x7030, value: Image::addr(pointee) + 8 }
    );
    assert_eq!(
        catch_object(&m, &h, &Match::Typed(base_ptr), &cxx(Image::addr(null), 0), 0x7000).unwrap(),
        CatchObject::Value { at: 0x7030, value: 0 }
    );
}

#[test]
fn thrown_tables_and_destructor() {
    let mut img = Image::new();
    let t = types(&mut img);
    let ti =
        throw_info(&mut img, 0, 0x7200, &[(0, t.derived, 0, -1, 0, 24, 0x7000), (0, t.base, 0, -1, 0, 16, 0x7100)]);
    let e = cxx(0x1234, ti);
    let m = img.mem();
    let th = Thrown::read(&m, &e).unwrap();
    assert_eq!(th.types.len(), 2);
    assert_eq!(th.types[1].size, 16);
    assert_eq!(th.destructor(&e).unwrap(), Some(BASE + 0x7200));
    let ti0 = throw_info(&mut img, 0, 0, &[(1, t.int, 0, -1, 0, 4, 0)]);
    let m = img.mem();
    let e0 = cxx(0x1234, ti0);
    assert_eq!(Thrown::read(&m, &e0).unwrap().destructor(&e0).unwrap(), None);
}

// ----- the catch blocks in progress ---------------------------------------------------

fn exc(object: u64) -> Exception {
    Exception::new(CXX_EXCEPTION, 0x81, &[u64::from(MAGIC_V1), object, 0x2000, BASE])
}

#[test]
fn catches_keep_states_and_the_current_exception() {
    let mut c = Catches::new();
    assert!(c.current().is_none());
    let a = c.push(0x9000, 0x100, -1, exc(1));
    assert_eq!(c.state_of(0x9000, 0x100), Some(-1));
    assert_eq!(c.state_of(0x9000, 0x200), None);
    assert_eq!(c.current().unwrap().object(), 1);
    // A nested catch block deeper in the stack.
    let b = c.push(0x8000, 0x300, 14, exc(2));
    assert_eq!(c.current().unwrap().object(), 2);
    assert_eq!(c.finish(b).unwrap().object(), 2);
    assert_eq!(c.current().unwrap().object(), 1);
    assert_eq!(c.finish(a).unwrap().object(), 1);
    assert!(c.entries().is_empty());
    assert_eq!(c.finish(a), None);
}

#[test]
fn a_new_catch_prunes_what_is_at_or_below_its_frame() {
    let mut c = Catches::new();
    let outer = c.push(0x9000, 0x100, -1, exc(1));
    let inner = c.push(0x8000, 0x300, 14, exc(2));
    // The inner catch block is left by a new exception taken by the
    // function at 0x9000.
    assert_eq!(c.abandon(inner, Some(&exc(3))).unwrap().object(), 2);
    assert_eq!(c.abandon(outer, Some(&exc(3))).unwrap().object(), 1);
    // Abandoned entries still give their frame's state.
    assert_eq!(c.state_of(0x8000, 0x300), Some(14));
    assert!(c.current().is_none());
    // Unwinding 0x8000 removes its entry; the target frame keeps its own.
    c.prune(0x8000, true);
    c.prune(0x9000, false);
    assert_eq!(c.entries().len(), 1);
    let n = c.push(0x9000, 0x100, -1, exc(3));
    assert_eq!(c.entries().len(), 1);
    assert_eq!(c.current().unwrap().object(), 3);
    assert_eq!(c.finish(n).unwrap().object(), 3);
    // Abandoning twice destroys once.
    let x = c.push(0x9000, 0x100, -1, exc(4));
    assert!(c.abandon(x, None).is_some());
    assert!(c.abandon(x, None).is_none());
}

#[test]
fn a_rethrow_passes_the_object_on() {
    let mut c = Catches::new();
    let a = c.push(0x8000, 0x100, -1, exc(5));
    // Taken by a C++ catch block further out: the same object, not
    // destroyed now.
    assert_eq!(c.abandon(a, Some(&exc(5))), None);
    let b = c.push(0x9000, 0x200, -1, exc(5));
    assert_eq!(c.entries().len(), 1);
    assert_eq!(c.finish(b).unwrap().object(), 5);
    // Taken by an __except outside: destroyed while the catch block is left.
    let a = c.push(0x8000, 0x100, -1, exc(6));
    assert_eq!(c.abandon(a, None).unwrap().object(), 6);
}

#[test]
fn a_rethrow_caught_inside_the_catch_block_keeps_the_object() {
    let mut c = Catches::new();
    let outer = c.push(0x9000, 0x100, -1, exc(7));
    // `throw;` inside the catch block, taken by a try block in it.
    let inner = c.push(0x8000, 0x200, 3, exc(7));
    assert_eq!(c.finish(inner), None);
    assert_eq!(c.finish(outer).unwrap().object(), 7);
}

#[test]
fn objects_without_throw_info_or_non_cxx_are_not_destroyed() {
    let mut c = Catches::new();
    let a = c.push(0x9000, 0x100, -1, Exception::new(0xE000_0001, 0x80, &[]));
    assert_eq!(c.finish(a), None);
    let b = c.push(0x9000, 0x100, -1, Exception::new(CXX_EXCEPTION, 0x81, &[u64::from(MAGIC_V1), 0, 0, 0]));
    assert_eq!(c.abandon(b, None), None);
}

// ----- robustness -------------------------------------------------------------------

/// A small deterministic generator (xorshift), so the fuzz test needs no
/// crate.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn random_tables_never_panic() {
    let mut rng = Rng(0x5EED_1993_0522);
    for round in 0..2000 {
        let mut bytes = vec![0u8; 512];
        for b in bytes.iter_mut() {
            *b = rng.next() as u8;
        }
        // Small counts and offsets often enough to reach the walks.
        let small = |r: &mut Rng| (r.next() % 600) as u32;
        let mut words = [0u32; 10];
        words[0] = [MAGIC_V1, MAGIC_V2, MAGIC_V3, rng.next() as u32][round % 4];
        for w in words.iter_mut().skip(1) {
            *w = if rng.next().is_multiple_of(4) { rng.next() as u32 } else { small(&mut rng) };
        }
        for (i, w) in words.iter().enumerate() {
            bytes[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
        }
        let m = SliceMemory { base: BASE, bytes: &bytes };
        let Ok(f) = Func::read(&m, BASE, 0) else { continue };
        let state = (rng.next() % 40) as i32 - 2;
        let begin = small(&mut rng);
        let e = Exception::new(
            CXX_EXCEPTION,
            0x81,
            &[u64::from(MAGIC_V1), BASE + u64::from(small(&mut rng)), BASE + u64::from(small(&mut rng)), BASE],
        );
        let seh = Exception::new(0xC000_0005, 0, &[0, 0]);
        let _ = f.state_at(rng.next() as u32);
        if let Ok(k) = frame_kind(&f, state, begin) {
            let _ = candidates(&f, k, state);
            let _ = search(&f, k, state, &e);
            let _ = search(&f, k, state, &seh);
            let _ = unwind_actions(&f, k, state, -1);
            let _ = parent_frame_offset(&f, k);
        }
        if let Ok(th) = Thrown::read(&m, &e) {
            for ct in &th.types {
                let _ = adjust(&m, e.object(), ct);
                let h = clause(rng.next() as u32, small(&mut rng), rng.next() as i32, 0);
                let _ = catch_object(&m, &h, &Match::Typed(*ct), &e, 0x7000);
                let _ = match_clause(&m, BASE, &h, &e, &th);
            }
        }
    }
}

#[test]
fn unreadable_memory_is_an_error() {
    let m = SliceMemory { base: BASE, bytes: &[] };
    assert!(matches!(FuncInfo::read(&m, BASE), Err(Bad::Unreadable(_))));
    assert!(matches!(Thrown::read(&m, &cxx(BASE, 0)), Err(Bad::Unreadable(_))));
    let _: &dyn Memory = &m;
}

#[test]
fn table_counts_are_bounded() {
    let mut img = Image::new();
    let rva = img.words(&[MAGIC_V3, -5i32 as u32, 0, u32::MAX, 0, u32::MAX, 0, 64, 0, 0]);
    let m = img.mem();
    let f = func(&m, rva);
    assert!(matches!(f.states(), Err(Bad::Count(-5))));
    assert!(matches!(f.try_block(0), Err(Bad::Count(_))));
    assert!(matches!(f.state_at(0), Err(Bad::Count(_))));
}

#[test]
fn a_negative_catchable_type_count_is_refused() {
    let mut img = Image::new();
    let t = types(&mut img);
    let ti = throw_info(&mut img, 0, 0, &[(1, t.int, 0, -1, 0, 4, 0)]);
    // The array's count is the word ThrowInfo's last field points at.
    let cta = u32::from_le_bytes(img.bytes[ti as usize + 12..ti as usize + 16].try_into().unwrap());
    img.put_i32(cta, -1);
    let m = img.mem();
    assert_eq!(Thrown::read(&m, &cxx(0x1234, ti)), Err(Bad::Count(-1)));
}
