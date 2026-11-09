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

/// A thread's apartment state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Apartment {
    model: Option<Model>,
    count: u32,
}

impl Apartment {
    pub const fn new() -> Apartment {
        Apartment { model: None, count: 0 }
    }

    /// Unpack the thread's TLS word.
    pub fn from_word(w: usize) -> Apartment {
        let count = (w >> 2) as u32;
        let model = match w & 3 {
            1 => Some(Model::Sta),
            2 => Some(Model::Mta),
            _ => None,
        };
        Apartment { model, count: if model.is_some() { count } else { 0 } }
    }

    /// The word for the TLS slot (0: not initialised).
    pub fn to_word(self) -> usize {
        match self.model {
            None => 0,
            Some(Model::Sta) => ((self.count as usize) << 2) | 1,
            Some(Model::Mta) => ((self.count as usize) << 2) | 2,
        }
    }

    pub fn model(self) -> Option<Model> {
        self.model
    }

    pub fn count(self) -> u32 {
        self.count
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
                self.count = self.count.saturating_add(1);
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
        } else {
            *self = Apartment::new();
        }
    }

    /// `CoGetApartmentType`: the type and qualifier, or `None`
    /// (`CO_E_NOTINITIALIZED`) when the thread has no apartment.
    pub fn apartment_type(self) -> Option<(i32, i32)> {
        self.model.map(|m| (m.apt_type(), APTTYPEQUALIFIER_NONE))
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
}
