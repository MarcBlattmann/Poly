//! A thread's apartment (Learn: "CoInitializeEx", "OleInitialize",
//! "RoInitialize", "CoGetApartmentType").
//!
//! One state per thread, kept in the block `TEB.ReservedForOle` points to
//! (sheet `TEB.ReservedForOle`): [`ComState`] is what that block holds, and
//! everything the DLL answers about the thread's apartment is derived from it.

use crate::hresult::{Hresult, RPC_E_CHANGED_MODE, S_FALSE, S_OK};

/// `COINIT_APARTMENTTHREADED` (`objbase.h`); `COINIT_MULTITHREADED` is 0.
/// The other bits (`COINIT_DISABLE_OLE1DDE` 4, `COINIT_SPEED_OVER_MEMORY` 8)
/// do not change the model.
pub const COINIT_APARTMENTTHREADED: u32 = 2;
/// `RO_INIT_SINGLETHREADED` / `RO_INIT_MULTITHREADED` (`roapi.h`).
pub const RO_INIT_SINGLETHREADED: i32 = 0;
pub const RO_INIT_MULTITHREADED: i32 = 1;

/// The bits `CoInitializeEx` accepts in `dwCoInit` (`COINIT_APARTMENTTHREADED` 2,
/// `COINIT_DISABLE_OLE1DDE` 4, `COINIT_SPEED_OVER_MEMORY` 8); any other bit is
/// `E_INVALIDARG` (sheet `CoInitializeEx`: 1, 0x10, 0x100 and `0xFFFFFFFF`).
pub const COINIT_VALID_BITS: u32 = 0xE;

/// Whether `dwCoInit` only holds defined bits.
pub fn coinit_is_valid(flags: u32) -> bool {
    flags & !COINIT_VALID_BITS == 0
}

/// `APTTYPE` and `APTTYPEQUALIFIER` (`objidl.h`).
pub const APTTYPE_STA: i32 = 0;
pub const APTTYPE_MTA: i32 = 1;
/// `APTTYPE_MAINSTA`: the first single-threaded apartment of the process.
pub const APTTYPE_MAINSTA: i32 = 3;
pub const APTTYPEQUALIFIER_NONE: i32 = 0;
/// `APTTYPEQUALIFIER_IMPLICIT_MTA` (`objidl.h`): the thread never initialised COM but lives in the
/// process's multithreaded apartment.
pub const APTTYPEQUALIFIER_IMPLICIT_MTA: i32 = 1;

/// Which thread owns the process's main STA (Learn, "APTTYPE": `APTTYPE_MAINSTA`
/// is the first STA a process makes; it is gone when that thread's last
/// initialisation is balanced, and the next STA thread becomes it). One word
/// per process: the owner's thread id, 0 for none.
#[derive(Debug)]
pub struct MainSta(core::sync::atomic::AtomicU32);

impl MainSta {
    pub const fn new() -> MainSta {
        MainSta(core::sync::atomic::AtomicU32::new(0))
    }

    /// A thread made its first STA initialisation: it is the main STA when none exists.
    pub fn claim(&self, tid: u32) {
        use core::sync::atomic::Ordering::{AcqRel, Acquire};
        let _ = self.0.compare_exchange(0, tid, AcqRel, Acquire);
    }

    /// A thread made its first STA initialisation: it becomes the main STA when there is none or the
    /// holder is gone (`alive` says whether a thread still runs: a thread can end without
    /// `CoUninitialize`). Whether `tid` is the main STA now.
    pub fn claim_if_gone(&self, tid: u32, alive: impl Fn(u32) -> bool) -> bool {
        use core::sync::atomic::Ordering::{AcqRel, Acquire};
        loop {
            let cur = self.0.load(Acquire);
            if cur == tid {
                return true;
            }
            if cur != 0 && alive(cur) {
                return false;
            }
            if self.0.compare_exchange(cur, tid, AcqRel, Acquire).is_ok() {
                return true;
            }
        }
    }

    /// A thread's STA ended.
    pub fn release(&self, tid: u32) {
        use core::sync::atomic::Ordering::{AcqRel, Acquire};
        let _ = self.0.compare_exchange(tid, 0, AcqRel, Acquire);
    }

    /// Whether `tid` owns the main STA.
    pub fn is_main(&self, tid: u32) -> bool {
        tid != 0 && self.0.load(core::sync::atomic::Ordering::Acquire) == tid
    }
}

impl Default for MainSta {
    fn default() -> Self {
        Self::new()
    }
}

/// How many threads of the process hold the multithreaded apartment (sheet `TEB.ReservedForOle`,
/// "implicit MTA"): while there is one, a thread that never initialised COM is in it too.
#[derive(Debug)]
pub struct MtaThreads(core::sync::atomic::AtomicU32);

impl MtaThreads {
    pub const fn new() -> MtaThreads {
        MtaThreads(core::sync::atomic::AtomicU32::new(0))
    }

    /// A thread entered the MTA by initialising COM.
    pub fn enter(&self) {
        self.0.fetch_add(1, core::sync::atomic::Ordering::AcqRel);
    }

    /// A thread's MTA initialisations were all undone.
    pub fn leave(&self) {
        use core::sync::atomic::Ordering::{AcqRel, Acquire};
        let _ = self.0.fetch_update(AcqRel, Acquire, |n| n.checked_sub(1));
    }

    /// Whether the process has a multithreaded apartment.
    pub fn any(&self) -> bool {
        self.0.load(core::sync::atomic::Ordering::Acquire) != 0
    }
}

impl Default for MtaThreads {
    fn default() -> Self {
        Self::new()
    }
}

/// What an activation of an unregistered class answers (sheets `CoCreateInstance` and
/// `CoGetClassObject`): `CO_E_NOTINITIALIZED` on a thread with no apartment,
/// `CLASS_E_NOAGGREGATION` when an outer unknown is given, else `REGDB_E_CLASSNOTREG`, whatever
/// the class context or the class id (the all-zero one too).
pub fn activation_failure(initialised: bool, has_outer: bool) -> Hresult {
    if !initialised {
        crate::hresult::CO_E_NOTINITIALIZED
    } else if has_outer {
        crate::hresult::CLASS_E_NOAGGREGATION
    } else {
        crate::hresult::REGDB_E_CLASSNOTREG
    }
}

/// The concurrency model of an initialised thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    /// A single-threaded apartment.
    Sta,
    /// The multithreaded apartment.
    Mta,
}

impl Model {
    /// The model `CoInitializeEx`'s `dwCoInit` asks for.
    pub fn from_coinit(flags: u32) -> Model {
        if flags & COINIT_APARTMENTTHREADED != 0 { Model::Sta } else { Model::Mta }
    }

    /// The model `RoInitialize`'s `RO_INIT_TYPE` asks for (anything but 0 is the MTA).
    pub fn from_ro(init_type: i32) -> Model {
        if init_type == RO_INIT_SINGLETHREADED { Model::Sta } else { Model::Mta }
    }

    /// The `APTTYPE` of the thread.
    pub fn apt_type(self) -> i32 {
        match self {
            Model::Sta => APTTYPE_STA,
            Model::Mta => APTTYPE_MTA,
        }
    }
}

/// The per-thread block `TEB.ReservedForOle` points to (`winternl.h`; sheet `TEB.ReservedForOle`).
/// Only three 32-bit fields are observable: the flags value, the count of initialisations not yet
/// undone and the count of `OleInitialize` calls not yet undone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockLayout {
    /// Where the pointer is in the TEB.
    pub teb_offset: usize,
    /// Offsets of the three fields in the block.
    pub flags: usize,
    pub count: usize,
    pub ole: usize,
    /// A dword of our own after the observable ones: non-zero once `OleInitialize` was ever called.
    pub ole_used: usize,
}

/// The 64-bit layout: `TEB.ReservedForOle` at 0x1758; flags +0x14, count +0x28, OLE count +0x2c.
pub const BLOCK_X64: BlockLayout = BlockLayout { teb_offset: 0x1758, flags: 0x14, count: 0x28, ole: 0x2C, ole_used: 0x30 };
/// The 32-bit layout (WOW64 and native 32-bit): at 0xF80; flags +0x0C, count +0x18, OLE count +0x1C.
pub const BLOCK_X86: BlockLayout = BlockLayout { teb_offset: 0xF80, flags: 0x0C, count: 0x18, ole: 0x1C, ole_used: 0x20 };
/// Bytes of a block (zeroed when made; the dwords other than the three above stay 0).
pub const BLOCK_SIZE: usize = 0x40;

/// Bits of the flags value (sheet: the numbers are the contract, the reading of the bits a help).
pub const FLAG_USED: u32 = 0x1;
pub const FLAG_NO_OLE1DDE: u32 = 0x40;
pub const FLAG_STA: u32 = 0x80;
pub const FLAG_MTA: u32 = 0x100;
pub const FLAG_WINRT: u32 = 0x20_0000;

/// What asks to initialise the thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitKind {
    /// `CoInitializeEx` with its `dwCoInit` (`CoInitialize` is 2).
    Com(u32),
    /// `OleInitialize`: a single-threaded apartment (and one OLE count).
    Ole,
    /// `RoInitialize` with its `RO_INIT_TYPE`.
    Ro(i32),
}

impl InitKind {
    /// The model it asks for.
    pub fn model(self) -> Model {
        match self {
            InitKind::Com(f) => Model::from_coinit(f),
            InitKind::Ole => Model::Sta,
            InitKind::Ro(t) => Model::from_ro(t),
        }
    }

    /// The flags value of a thread this call initialises first (sheet table).
    fn first_value(self) -> u32 {
        match self {
            InitKind::Com(f) => match Model::from_coinit(f) {
                Model::Sta => FLAG_USED | FLAG_STA | if f & 4 != 0 { FLAG_NO_OLE1DDE } else { 0 },
                Model::Mta => FLAG_USED | FLAG_MTA | FLAG_NO_OLE1DDE,
            },
            InitKind::Ole => FLAG_USED | FLAG_STA,
            // The Windows Runtime's single-threaded kind behaves like the DISABLE_OLE1DDE one.
            InitKind::Ro(t) => {
                FLAG_WINRT
                    | FLAG_USED
                    | FLAG_NO_OLE1DDE
                    | match Model::from_ro(t) {
                        Model::Sta => FLAG_STA,
                        Model::Mta => FLAG_MTA,
                    }
            }
        }
    }
}

/// What the block of a thread holds: the one truth about its apartment (`CoGetApartmentType` is
/// derived from it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ComState {
    /// The 32-bit value at the flags offset.
    pub flags: u32,
    /// Initialisations not yet undone.
    pub count: u32,
    /// `OleInitialize` calls not yet undone.
    pub ole: u32,
    /// `OleInitialize` was ever called on this thread (`RegisterDragDrop` tells "OLE not initialised"
    /// from "OLE uninitialised again" by it). Kept in a dword of our own, not in the sheet's.
    pub ole_used: bool,
}

impl ComState {
    /// A block just made: all zero.
    pub const fn new() -> ComState {
        ComState { flags: 0, count: 0, ole: 0, ole_used: false }
    }

    /// The model of an initialised thread; `None` when it has no apartment of its own.
    pub fn model(self) -> Option<Model> {
        if self.count == 0 {
            None
        } else if self.flags & FLAG_STA != 0 {
            Some(Model::Sta)
        } else if self.flags & FLAG_MTA != 0 {
            Some(Model::Mta)
        } else {
            None
        }
    }

    /// The thread used COM in a way that fails (`CoCreateInstance` on an uninitialised thread):
    /// the block exists and holds 0x1.
    pub fn touch(&mut self) {
        self.flags |= FLAG_USED;
    }

    /// `CoInitializeEx` / `OleInitialize` / `RoInitialize`: `S_OK` for the first call, `S_FALSE`
    /// when the thread already is in this model (the count goes up), `RPC_E_CHANGED_MODE` (nothing
    /// changes) for another model.
    pub fn initialize(&mut self, kind: InitKind) -> Hresult {
        let want = kind.model();
        let hr = match self.model() {
            None => {
                self.flags = kind.first_value();
                self.count = 1;
                self.ole = 0;
                S_OK
            }
            Some(m) if m == want => {
                self.count = self.count.saturating_add(1);
                S_FALSE
            }
            Some(_) => return RPC_E_CHANGED_MODE,
        };
        if kind == InitKind::Ole {
            // The first OleInitialize of the thread is S_OK, also on a thread CoInitializeEx made.
            let first = self.ole == 0;
            self.ole = self.ole.saturating_add(1);
            self.ole_used = true;
            return if first { S_OK } else { S_FALSE };
        }
        hr
    }

    /// `CoUninitialize` / `RoUninitialize`: balances one call. The last one leaves the block as it is
    /// with the flags value 0x1; on a thread that is not initialised nothing changes.
    pub fn uninitialize(&mut self) {
        if self.count == 0 {
            return;
        }
        self.count -= 1;
        if self.ole > self.count {
            self.ole = self.count;
        }
        if self.count == 0 {
            self.flags = FLAG_USED;
            self.ole = 0;
        }
    }

    /// `OleUninitialize`: balances one `OleInitialize` (and its share of the apartment); `true` when
    /// that was the last one (the drop targets go). A call without OLE does nothing.
    pub fn ole_uninitialize(&mut self) -> bool {
        if self.ole == 0 {
            return false;
        }
        let ole = self.ole - 1;
        self.uninitialize();
        if self.model().is_some() {
            self.ole = ole;
        }
        ole == 0
    }

    /// `CoGetApartmentType`: the type and qualifier, or `None` (`CO_E_NOTINITIALIZED`). An STA
    /// reports `APTTYPE_MAINSTA` (3) when `main`, the first STA of the process; a thread with no
    /// apartment of its own is in the implicit MTA (qualifier 1) while `mta_exists`.
    pub fn apartment_type_in(self, main: bool, mta_exists: bool) -> Option<(i32, i32)> {
        match self.model() {
            Some(Model::Sta) if main => Some((APTTYPE_MAINSTA, APTTYPEQUALIFIER_NONE)),
            Some(m) => Some((m.apt_type(), APTTYPEQUALIFIER_NONE)),
            None if mta_exists => Some((APTTYPE_MTA, APTTYPEQUALIFIER_IMPLICIT_MTA)),
            None => None,
        }
    }

    /// Whether COM calls work on the thread: its own apartment or the implicit MTA.
    pub fn is_usable(self, mta_exists: bool) -> bool {
        self.model().is_some() || mta_exists
    }

    /// Read a block at `block`.
    ///
    /// # Safety
    /// `block` points to a readable block of [`BLOCK_SIZE`] bytes laid out as `layout`.
    pub unsafe fn read(block: *const u8, layout: BlockLayout) -> ComState {
        // SAFETY: the caller's block.
        unsafe {
            ComState {
                flags: block.add(layout.flags).cast::<u32>().read_unaligned(),
                count: block.add(layout.count).cast::<u32>().read_unaligned(),
                ole: block.add(layout.ole).cast::<u32>().read_unaligned(),
                ole_used: block.add(layout.ole_used).cast::<u32>().read_unaligned() != 0,
            }
        }
    }

    /// Write the three fields into `block`.
    ///
    /// # Safety
    /// `block` points to a writable block of [`BLOCK_SIZE`] bytes laid out as `layout`.
    pub unsafe fn write(self, block: *mut u8, layout: BlockLayout) {
        // SAFETY: the caller's block.
        unsafe {
            block.add(layout.flags).cast::<u32>().write_unaligned(self.flags);
            block.add(layout.count).cast::<u32>().write_unaligned(self.count);
            block.add(layout.ole).cast::<u32>().write_unaligned(self.ole);
            block.add(layout.ole_used).cast::<u32>().write_unaligned(u32::from(self.ole_used));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init(s: &mut ComState, k: InitKind) -> Hresult {
        s.initialize(k)
    }

    /// The states of the sheet `TEB.ReservedForOle`: the flags value after a first call.
    #[test]
    fn first_call_values() {
        for (kind, value) in [
            (InitKind::Com(2), 0x81),
            (InitKind::Com(6), 0xC1),
            (InitKind::Com(0xA), 0x81),
            (InitKind::Com(0xE), 0xC1),
            (InitKind::Com(0), 0x141),
            (InitKind::Com(4), 0x141),
            (InitKind::Com(8), 0x141),
            (InitKind::Com(0xC), 0x141),
            (InitKind::Ole, 0x81),
            (InitKind::Ro(RO_INIT_SINGLETHREADED), 0x2000C1),
            (InitKind::Ro(RO_INIT_MULTITHREADED), 0x200141),
        ] {
            let mut s = ComState::new();
            assert_eq!(init(&mut s, kind), S_OK, "{kind:?}");
            assert_eq!((s.flags, s.count), (value, 1), "{kind:?}");
        }
    }

    #[test]
    fn nested_calls_and_other_models() {
        let mut s = ComState::new();
        assert_eq!(init(&mut s, InitKind::Com(2)), S_OK);
        assert_eq!(init(&mut s, InitKind::Com(2)), S_FALSE);
        assert_eq!((s.flags, s.count), (0x81, 2));
        assert_eq!(init(&mut s, InitKind::Com(0)), RPC_E_CHANGED_MODE);
        assert_eq!(init(&mut s, InitKind::Ro(RO_INIT_MULTITHREADED)), RPC_E_CHANGED_MODE);
        assert_eq!((s.flags, s.count), (0x81, 2));
        // RoInitialize(STA) after CoInitializeEx(2): S_FALSE, the value stays, no WinRT bit.
        assert_eq!(init(&mut s, InitKind::Ro(RO_INIT_SINGLETHREADED)), S_FALSE);
        assert_eq!((s.flags, s.count), (0x81, 3));
    }

    #[test]
    fn the_last_uninitialise_leaves_one_and_the_block_is_reused() {
        let mut s = ComState::new();
        init(&mut s, InitKind::Com(6));
        init(&mut s, InitKind::Com(6));
        s.uninitialize();
        assert_eq!((s.flags, s.count), (0xC1, 1));
        assert!(s.model().is_some());
        s.uninitialize();
        assert_eq!((s.flags, s.count, s.ole), (0x1, 0, 0));
        assert_eq!(s.model(), None);
        s.uninitialize(); // an extra one changes nothing
        assert_eq!((s.flags, s.count), (0x1, 0));
        // Either model again.
        assert_eq!(init(&mut s, InitKind::Com(0)), S_OK);
        assert_eq!((s.flags, s.count), (0x141, 1));
        s.uninitialize();
        assert_eq!(init(&mut s, InitKind::Com(2)), S_OK);
        assert_eq!((s.flags, s.count), (0x81, 1));
    }

    #[test]
    fn ole_counts() {
        let mut s = ComState::new();
        assert_eq!(init(&mut s, InitKind::Ole), S_OK);
        assert_eq!((s.flags, s.count, s.ole), (0x81, 1, 1));
        assert_eq!(init(&mut s, InitKind::Ole), S_FALSE);
        assert_eq!((s.count, s.ole), (2, 2));
        s.ole_uninitialize();
        assert_eq!((s.flags, s.count, s.ole), (0x81, 1, 1));
        s.ole_uninitialize();
        assert_eq!((s.flags, s.count, s.ole), (0x1, 0, 0));
        // OleInitialize on a thread made by CoInitializeEx(2): the first OLE call is S_OK.
        init(&mut s, InitKind::Com(2));
        assert_eq!(init(&mut s, InitKind::Ole), S_OK);
        assert_eq!((s.flags, s.count, s.ole), (0x81, 2, 1));
        s.ole_uninitialize();
        assert_eq!((s.count, s.ole), (1, 0));
        s.uninitialize();
        assert_eq!(s.flags, 0x1);
        // On an MTA thread OleInitialize changes nothing.
        let mut m = ComState::new();
        init(&mut m, InitKind::Com(0));
        assert_eq!(init(&mut m, InitKind::Ole), RPC_E_CHANGED_MODE);
        assert_eq!((m.flags, m.count, m.ole), (0x141, 1, 0));
        assert!(!m.ole_used);
    }

    #[test]
    fn ole_is_a_state_of_its_own() {
        let mut a = ComState::new();
        assert_eq!(init(&mut a, InitKind::Ole), S_OK);
        assert_eq!(init(&mut a, InitKind::Ole), S_FALSE);
        assert!(!a.ole_uninitialize());
        assert!(a.ole_uninitialize());
        assert_eq!(a.model(), None);
        assert!(a.ole_used);
        // An unmatched call changes nothing, and the next OleInitialize is the first again.
        assert!(!a.ole_uninitialize());
        assert_eq!(a.count, 0);
        assert_eq!(init(&mut a, InitKind::Ole), S_OK);
        assert!(a.ole_uninitialize());
        // On a thread that did CoInitializeEx: the apartment stays after OleUninitialize.
        let mut b = ComState::new();
        assert_eq!(init(&mut b, InitKind::Com(2)), S_OK);
        assert_eq!(init(&mut b, InitKind::Ole), S_OK);
        assert!(b.ole_uninitialize());
        assert_eq!(b.model(), Some(Model::Sta));
        b.uninitialize();
        assert_eq!(b.model(), None);
    }

    #[test]
    fn a_failing_creation_gives_the_block_value_one() {
        let mut s = ComState::new();
        s.touch();
        assert_eq!((s.flags, s.count, s.ole), (0x1, 0, 0));
        assert_eq!(s.model(), None);
        assert_eq!(s.apartment_type_in(true, false), None);
        init(&mut s, InitKind::Com(2));
        assert_eq!(s.flags, 0x81);
    }

    #[test]
    fn apartment_type_is_derived_from_the_block() {
        let mut s = ComState::new();
        assert_eq!(s.apartment_type_in(true, false), None);
        init(&mut s, InitKind::Com(2));
        assert_eq!(s.apartment_type_in(true, false), Some((APTTYPE_MAINSTA, 0)));
        assert_eq!(s.apartment_type_in(false, false), Some((APTTYPE_STA, 0)));
        let mut m = ComState::new();
        init(&mut m, InitKind::Com(0));
        assert_eq!(m.apartment_type_in(true, true), Some((APTTYPE_MTA, 0)));
        // A thread with no apartment while the process has an MTA: the implicit one.
        let n = ComState::new();
        assert_eq!(n.apartment_type_in(false, true), Some((APTTYPE_MTA, APTTYPEQUALIFIER_IMPLICIT_MTA)));
        assert!(n.is_usable(true));
        assert!(!n.is_usable(false));
    }

    #[test]
    fn block_round_trips_in_both_layouts() {
        for layout in [BLOCK_X64, BLOCK_X86] {
            let mut mem = [0u8; BLOCK_SIZE];
            let s = ComState { flags: 0x2000C1, count: 3, ole: 2, ole_used: true };
            // SAFETY: a block-sized buffer.
            let back = unsafe {
                s.write(mem.as_mut_ptr(), layout);
                ComState::read(mem.as_ptr(), layout)
            };
            assert_eq!(back, s);
        }
        assert_eq!(BLOCK_X64.teb_offset, 0x1758);
        assert_eq!(BLOCK_X86.teb_offset, 0xF80);
    }

    #[test]
    fn the_mta_thread_count() {
        let m = MtaThreads::new();
        assert!(!m.any());
        m.enter();
        m.enter();
        m.leave();
        assert!(m.any());
        m.leave();
        m.leave(); // never below zero
        assert!(!m.any());
    }

    #[test]
    fn activation_of_an_unregistered_class() {
        use crate::hresult::*;
        assert_eq!(activation_failure(false, false), CO_E_NOTINITIALIZED);
        assert_eq!(activation_failure(false, true), CO_E_NOTINITIALIZED);
        assert_eq!(activation_failure(true, false), REGDB_E_CLASSNOTREG);
        assert_eq!(activation_failure(true, true), CLASS_E_NOAGGREGATION);
    }

    #[test]
    fn coinit_accepts_only_the_defined_bits() {
        for ok in [0, 2, 4, 6, 8, 10, 12, 14] {
            assert!(coinit_is_valid(ok), "{ok:#x}");
        }
        for bad in [1, 3, 0x10, 0x100, 0x8000_0000, 0xFFFF_FFFF] {
            assert!(!coinit_is_valid(bad), "{bad:#x}");
        }
    }

    #[test]
    fn the_first_sta_is_the_main_one_until_it_ends() {
        let m = MainSta::new();
        assert!(!m.is_main(7));
        m.claim(7);
        m.claim(9);
        assert!(m.is_main(7));
        assert!(!m.is_main(9));
        m.release(9);
        assert!(m.is_main(7));
        m.release(7);
        assert!(!m.is_main(7));
        m.claim(9);
        assert!(m.is_main(9));
        assert!(!m.is_main(0));
    }

    #[test]
    fn a_main_sta_that_ended_without_uninitialising_is_replaced() {
        let m = MainSta::new();
        assert!(m.claim_if_gone(7, |_| true));
        assert!(m.claim_if_gone(7, |_| true));
        assert!(!m.claim_if_gone(9, |t| t == 7));
        assert!(m.is_main(7));
        assert!(m.claim_if_gone(9, |_| false));
        assert!(m.is_main(9));
    }

    #[test]
    fn models_follow_the_flags() {
        assert_eq!(Model::from_coinit(0), Model::Mta);
        assert_eq!(Model::from_coinit(COINIT_APARTMENTTHREADED | 4), Model::Sta);
        assert_eq!(Model::from_ro(RO_INIT_SINGLETHREADED), Model::Sta);
        assert_eq!(Model::from_ro(RO_INIT_MULTITHREADED), Model::Mta);
    }
}
