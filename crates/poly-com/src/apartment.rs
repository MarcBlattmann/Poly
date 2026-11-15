//! A thread's apartment (Learn: "CoInitializeEx", "OleInitialize",
//! "RoInitialize", "CoGetApartmentType").
//!
//! One state per thread, packed in a `usize` so the DLL can keep it in a
//! TLS slot: the model and the number of successful initialisations.

use crate::hresult::{Hresult, RPC_E_CHANGED_MODE, S_FALSE, S_OK};

/// `COINIT_APARTMENTTHREADED` (`objbase.h`); `COINIT_MULTITHREADED` is 0.
/// The other bits (`COINIT_DISABLE_OLE1DDE` 4, `COINIT_SPEED_OVER_MEMORY` 8)
/// do not change the model.
pub const COINIT_APARTMENTTHREADED: u32 = 2;
/// `RO_INIT_SINGLETHREADED` / `RO_INIT_MULTITHREADED` (`roapi.h`).
pub const RO_INIT_SINGLETHREADED: i32 = 0;
pub const RO_INIT_MULTITHREADED: i32 = 1;

/// `APTTYPE` and `APTTYPEQUALIFIER` (`objidl.h`).
pub const APTTYPE_STA: i32 = 0;
pub const APTTYPE_MTA: i32 = 1;
pub const APTTYPEQUALIFIER_NONE: i32 = 0;

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

/// `APTTYPE_MAINSTA` (`objidl.h`): the first single-threaded apartment of the process, while it lives.
pub const APTTYPE_MAINSTA: i32 = 3;
/// `APTTYPEQUALIFIER_IMPLICIT_MTA`: a thread without COM in a process that has the MTA.
pub const APTTYPEQUALIFIER_IMPLICIT_MTA: i32 = 1;

/// `CoInitializeEx`'s `dwCoInit` bits that mean something (`COINIT_APARTMENTTHREADED` 2,
/// `COINIT_DISABLE_OLE1DDE` 4, `COINIT_SPEED_OVER_MEMORY` 8); any other bit is `E_INVALIDARG`.
pub const COINIT_VALID: u32 = 0xE;

/// A thread's apartment state: the model, how many initialisations are open, whether it is the
/// process's main STA, and how many of them are `OleInitialize` (OLE is a state of its own on
/// top of the apartment).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Apartment {
    model: Option<Model>,
    count: u32,
    main: bool,
    ole: u32,
    ole_used: bool,
}

/// The widths of the packed word: model 2 bits, main 1, ever-OLE 1, count 14, OLE count 14.
const COUNT_MAX: u32 = 0x3FFF;

impl Apartment {
    pub const fn new() -> Apartment {
        Apartment { model: None, count: 0, main: false, ole: 0, ole_used: false }
    }

    /// Unpack the thread's TLS word.
    pub fn from_word(w: usize) -> Apartment {
        let model = match w & 3 {
            1 => Some(Model::Sta),
            2 => Some(Model::Mta),
            _ => None,
        };
        let ole_used = w & 8 != 0;
        if model.is_none() {
            return Apartment { ole_used, ..Apartment::new() };
        }
        Apartment {
            model,
            count: ((w >> 4) as u32) & COUNT_MAX,
            main: w & 4 != 0,
            ole: ((w >> 18) as u32) & COUNT_MAX,
            ole_used,
        }
    }

    /// The word for the TLS slot (0: nothing to remember).
    pub fn to_word(self) -> usize {
        let model = match self.model {
            None => 0,
            Some(Model::Sta) => 1,
            Some(Model::Mta) => 2,
        };
        let used = if self.ole_used { 8 } else { 0 };
        if self.model.is_none() {
            return used;
        }
        model | used | if self.main { 4 } else { 0 } | ((self.count as usize) << 4) | ((self.ole as usize) << 18)
    }

    pub fn model(self) -> Option<Model> {
        self.model
    }

    pub fn count(self) -> u32 {
        self.count
    }

    /// Whether this thread is the process's main STA.
    pub fn is_main(self) -> bool {
        self.main
    }

    /// Make the thread the main STA (the caller decides: the first STA of the process).
    pub fn set_main(&mut self, main: bool) {
        self.main = main && self.model == Some(Model::Sta);
    }

    /// Whether `OleInitialize` was ever called on this thread (what `RegisterDragDrop` tells
    /// "OLE not initialised" from "OLE uninitialised again" by).
    pub fn ole_used(self) -> bool {
        self.ole_used
    }

    /// How many `OleInitialize` calls are open.
    pub fn ole_count(self) -> u32 {
        self.ole
    }

    /// `CoInitializeEx` / `RoInitialize`: `S_OK` for the first call,
    /// `S_FALSE` when the thread was already initialised in this model,
    /// `RPC_E_CHANGED_MODE` (state unchanged) for another model.
    pub fn initialize(&mut self, want: Model) -> Hresult {
        match self.model {
            None => {
                self.model = Some(want);
                self.count = 1;
                S_OK
            }
            Some(m) if m == want => {
                self.count = (self.count + 1).min(COUNT_MAX);
                S_FALSE
            }
            Some(_) => RPC_E_CHANGED_MODE,
        }
    }

    /// `CoUninitialize` / `RoUninitialize`: balances one successful call; a
    /// call on a thread that is not initialised does nothing.
    pub fn uninitialize(&mut self) {
        if self.count > 1 {
            self.count -= 1;
            self.ole = self.ole.min(self.count);
        } else {
            *self = Apartment { ole_used: self.ole_used, ..Apartment::new() };
        }
    }

    /// `OleInitialize`: `S_OK` for the first call (also on a thread that did `CoInitializeEx` for the
    /// STA), `S_FALSE` for a nested one, `RPC_E_CHANGED_MODE` on an MTA thread.
    pub fn ole_initialize(&mut self) -> Hresult {
        if self.model == Some(Model::Mta) {
            return RPC_E_CHANGED_MODE;
        }
        self.initialize(Model::Sta);
        let first = self.ole == 0;
        self.ole = (self.ole + 1).min(COUNT_MAX);
        self.ole_used = true;
        if first { S_OK } else { S_FALSE }
    }

    /// `OleUninitialize`: balances one `OleInitialize` (and its share of the apartment); `true` when
    /// that was the last one (the drop targets go). A call without OLE does nothing.
    pub fn ole_uninitialize(&mut self) -> bool {
        if self.ole == 0 {
            return false;
        }
        let ole = self.ole - 1;
        self.uninitialize();
        if self.model.is_some() {
            self.ole = ole;
        }
        ole == 0
    }

    /// `CoGetApartmentType`: the type and qualifier, or `None`
    /// (`CO_E_NOTINITIALIZED`) when the thread has no apartment.
    pub fn apartment_type(self) -> Option<(i32, i32)> {
        self.model
            .map(|m| (if self.main && m == Model::Sta { APTTYPE_MAINSTA } else { m.apt_type() }, APTTYPEQUALIFIER_NONE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_call_ok_same_model_false_other_model_changed() {
        let mut a = Apartment::new();
        assert_eq!(a.initialize(Model::from_coinit(COINIT_APARTMENTTHREADED)), S_OK);
        assert_eq!(a.initialize(Model::Sta), S_FALSE);
        assert_eq!(a.initialize(Model::Mta), RPC_E_CHANGED_MODE);
        assert_eq!(a.count(), 2);
        a.uninitialize();
        assert_eq!(a.model(), Some(Model::Sta));
        a.uninitialize();
        assert_eq!(a.model(), None);
        // The model can change once the thread is fully uninitialised.
        assert_eq!(a.initialize(Model::Mta), S_OK);
    }

    #[test]
    fn uninitialize_without_initialize_is_harmless() {
        let mut a = Apartment::new();
        a.uninitialize();
        assert_eq!(a, Apartment::new());
    }

    #[test]
    fn word_round_trip() {
        let mut a = Apartment::new();
        assert_eq!(a.to_word(), 0);
        a.initialize(Model::Mta);
        a.initialize(Model::Mta);
        let b = Apartment::from_word(a.to_word());
        assert_eq!(b, a);
        assert_eq!(b.apartment_type(), Some((APTTYPE_MTA, APTTYPEQUALIFIER_NONE)));
        assert_eq!(Apartment::from_word(0).apartment_type(), None);
    }

    #[test]
    fn models_follow_the_flags() {
        assert_eq!(Model::from_coinit(0), Model::Mta);
        assert_eq!(Model::from_coinit(COINIT_APARTMENTTHREADED | 4), Model::Sta);
        assert_eq!(Model::from_ro(RO_INIT_SINGLETHREADED), Model::Sta);
        assert_eq!(Model::from_ro(RO_INIT_MULTITHREADED), Model::Mta);
    }

    #[test]
    fn the_main_sta_and_ole_are_states_of_their_own() {
        let mut a = Apartment::new();
        assert_eq!(a.ole_initialize(), S_OK);
        a.set_main(true);
        assert_eq!(a.apartment_type(), Some((APTTYPE_MAINSTA, 0)));
        assert_eq!(a.ole_initialize(), S_FALSE);
        assert_eq!(Apartment::from_word(a.to_word()), a);
        assert!(!a.ole_uninitialize());
        assert!(a.ole_uninitialize());
        assert_eq!(a.model(), None);
        assert!(a.ole_used());
        assert!(Apartment::from_word(a.to_word()).ole_used());
        // An unmatched call changes nothing, and the next OleInitialize is the first again.
        assert!(!a.ole_uninitialize());
        assert_eq!(a.ole_initialize(), S_OK);
        assert!(a.ole_uninitialize());
        // On a thread that did CoInitializeEx: S_OK, the apartment stays after OleUninitialize.
        let mut b = Apartment::new();
        assert_eq!(b.initialize(Model::Sta), S_OK);
        assert_eq!(b.ole_initialize(), S_OK);
        assert!(b.ole_uninitialize());
        assert_eq!(b.model(), Some(Model::Sta));
        b.uninitialize();
        assert_eq!(b.model(), None);
        // The MTA refuses OLE.
        let mut c = Apartment::new();
        c.initialize(Model::Mta);
        assert_eq!(c.ole_initialize(), RPC_E_CHANGED_MODE);
        assert_eq!(c.ole_count(), 0);
        assert!(!c.ole_used());
    }
}
