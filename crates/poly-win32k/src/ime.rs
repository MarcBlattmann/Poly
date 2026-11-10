//! Input contexts without an IME (ADR 0172): the contexts of `imm32.dll`,
//! one default per thread, and the window-to-context association. Sheets:
//! `specs/windows/imm32.dll/Imm*.yaml`. No IME is installed, so a context
//! only keeps the forms the application stored (composition window,
//! candidate windows); its open status and composition are always empty.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

type Key = (u32, u32);

pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_INVALID_HANDLE: u32 = 6;

/// What a window is associated with: the thread's default context, none,
/// or an own context.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Assoc {
    #[default]
    Default,
    None,
    Own(u32),
}

impl Assoc {
    /// The wire form: 0xFFFF_FFFF default, 0 none, else the handle.
    pub fn code(self) -> u32 {
        match self {
            Assoc::Default => u32::MAX,
            Assoc::None => 0,
            Assoc::Own(h) => h,
        }
    }

    pub fn from_code(c: u32) -> Assoc {
        match c {
            u32::MAX => Assoc::Default,
            0 => Assoc::None,
            h => Assoc::Own(h),
        }
    }
}

crate::wire_enum! {
    /// One operation on the contexts.
    pub enum ImeOp {
        /// A context of the thread; a `Val` with its handle (the default
        /// of `tid_of` when `default`).
        1 => Create {},
        2 => Destroy { himc: u32 },
        /// Whether a context exists (`Ok` or `Err` 6).
        3 => Check { himc: u32 },
        /// The default context of the thread `key` (created when missing).
        4 => DefaultOf { pid: u32, tid: u32 },
        /// The context a window with association `code` of thread `pid`/`tid` has: a `Val`.
        5 => Resolve { code: u32, pid: u32, tid: u32 },
        /// Whether `himc` may be associated with a window of the thread: `Ok`, `Err`.
        6 => MayAssociate { himc: u32, pid: u32, tid: u32 },
        /// Store (`set`) or read a form: kind 0 composition window, 1.. candidate window 0..3.
        7 => Form { himc: u32, kind: u8, set: bool, bytes: Vec<u8> },
    }
}

crate::wire_enum! {
    /// The answer to an [`ImeOp`].
    pub enum ImeRes {
        1 => Ok {},
        2 => Err { code: u32 },
        3 => Val { value: u32 },
        4 => Bytes { bytes: Vec<u8> },
        /// A form that was never stored.
        5 => Unset {},
    }
}

#[derive(Clone, Debug, Default)]
struct Ctx {
    owner: Key,
    default: bool,
    forms: [Option<Vec<u8>>; 5],
}

/// All contexts of a session.
#[derive(Clone, Debug, Default)]
pub struct Ime {
    ctxs: BTreeMap<u32, Ctx>,
    defaults: BTreeMap<Key, u32>,
    next: u32,
}

impl Ime {
    fn new_ctx(&mut self, owner: Key, default: bool) -> u32 {
        self.next += 1;
        let h = 0x00E1_0000 + self.next * 16;
        self.ctxs.insert(h, Ctx { owner, default, forms: Default::default() });
        h
    }

    fn default_of(&mut self, key: Key) -> u32 {
        if let Some(&h) = self.defaults.get(&key) {
            return h;
        }
        let h = self.new_ctx(key, true);
        self.defaults.insert(key, h);
        h
    }

    /// Run `op` for thread `key`.
    pub fn run(&mut self, key: Key, op: ImeOp) -> ImeRes {
        match op {
            ImeOp::Create {} => ImeRes::Val { value: self.new_ctx(key, false) },
            ImeOp::Destroy { himc } => match self.ctxs.get(&himc) {
                None => ImeRes::Err { code: ERROR_INVALID_HANDLE },
                Some(c) if c.default => ImeRes::Err { code: 0 },
                Some(c) if c.owner != key => ImeRes::Err { code: ERROR_INVALID_HANDLE },
                Some(_) => {
                    self.ctxs.remove(&himc);
                    ImeRes::Ok {}
                }
            },
            ImeOp::Check { himc } => {
                if self.ctxs.contains_key(&himc) {
                    ImeRes::Ok {}
                } else {
                    ImeRes::Err { code: ERROR_INVALID_HANDLE }
                }
            }
            ImeOp::DefaultOf { pid, tid } => ImeRes::Val { value: self.default_of((pid, tid)) },
            ImeOp::Resolve { code, pid, tid } => match Assoc::from_code(code) {
                Assoc::None => ImeRes::Val { value: 0 },
                Assoc::Own(h) if self.ctxs.contains_key(&h) => ImeRes::Val { value: h },
                _ => ImeRes::Val { value: self.default_of((pid, tid)) },
            },
            ImeOp::MayAssociate { himc, pid, tid } => {
                let Some(c) = self.ctxs.get(&himc) else { return ImeRes::Err { code: ERROR_INVALID_HANDLE } };
                if c.owner != (pid, tid) {
                    return ImeRes::Err { code: ERROR_ACCESS_DENIED };
                }
                ImeRes::Val { value: u32::from(c.default) }
            }
            ImeOp::Form { himc, kind, set, bytes } => {
                let Some(c) = self.ctxs.get_mut(&himc) else { return ImeRes::Err { code: ERROR_INVALID_HANDLE } };
                let Some(slot) = c.forms.get_mut(usize::from(kind)) else { return ImeRes::Unset {} };
                if set {
                    *slot = Some(bytes);
                    ImeRes::Ok {}
                } else {
                    match slot {
                        Some(b) => ImeRes::Bytes { bytes: b.clone() },
                        None => ImeRes::Unset {},
                    }
                }
            }
        }
    }

    /// A thread ended: its contexts go.
    pub fn thread_gone(&mut self, key: Key) {
        self.ctxs.retain(|_, c| c.owner != key);
        self.defaults.remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contexts_live_and_die_with_their_thread() {
        let mut ime = Ime::default();
        let t = (1, 10);
        let ImeRes::Val { value: d } = ime.run(t, ImeOp::DefaultOf { pid: 1, tid: 10 }) else { panic!() };
        assert_eq!(ime.run(t, ImeOp::DefaultOf { pid: 1, tid: 10 }), ImeRes::Val { value: d });
        let ImeRes::Val { value: a } = ime.run(t, ImeOp::Create {}) else { panic!() };
        assert_ne!(a, d);
        assert_eq!(ime.run(t, ImeOp::Destroy { himc: d }), ImeRes::Err { code: 0 });
        assert_eq!(ime.run(t, ImeOp::MayAssociate { himc: a, pid: 1, tid: 11 }), ImeRes::Err { code: 5 });
        assert_eq!(ime.run(t, ImeOp::Resolve { code: a, pid: 1, tid: 10 }), ImeRes::Val { value: a });
        assert_eq!(ime.run(t, ImeOp::Destroy { himc: a }), ImeRes::Ok {});
        // A destroyed own context falls back to the default.
        assert_eq!(ime.run(t, ImeOp::Resolve { code: a, pid: 1, tid: 10 }), ImeRes::Val { value: d });
        assert_eq!(ime.run(t, ImeOp::Destroy { himc: a }), ImeRes::Err { code: 6 });
        ime.thread_gone(t);
        assert_eq!(ime.run(t, ImeOp::Check { himc: d }), ImeRes::Err { code: 6 });
    }

    /// ImmSetCompositionWindow.yaml / ImmSetCandidateWindow.yaml: NULL, a
    /// made-up and a destroyed context are ERROR_INVALID_HANDLE; the forms
    /// of a context are independent, kept as given and unset until stored.
    #[test]
    fn forms_are_per_context_and_bad_contexts_are_invalid_handles() {
        let mut ime = Ime::default();
        let t = (1, 10);
        let ImeRes::Val { value: d } = ime.run(t, ImeOp::DefaultOf { pid: 1, tid: 10 }) else { panic!() };
        let ImeRes::Val { value: a } = ime.run(t, ImeOp::Create {}) else { panic!() };
        for bad in [0u32, 0x1234_5678] {
            assert_eq!(ime.run(t, ImeOp::Check { himc: bad }), ImeRes::Err { code: 6 });
            let set = ImeOp::Form { himc: bad, kind: 0, set: true, bytes: alloc::vec![1; 28] };
            assert_eq!(ime.run(t, set), ImeRes::Err { code: 6 });
        }
        let get = |ime: &mut Ime, himc, kind| ime.run(t, ImeOp::Form { himc, kind, set: false, bytes: Vec::new() });
        for kind in 0..5 {
            assert_eq!(get(&mut ime, d, kind), ImeRes::Unset {});
        }
        for kind in 0..5u8 {
            let bytes = alloc::vec![kind + 1; 32];
            assert_eq!(ime.run(t, ImeOp::Form { himc: a, kind, set: true, bytes }), ImeRes::Ok {});
        }
        for kind in 0..5u8 {
            assert_eq!(get(&mut ime, a, kind), ImeRes::Bytes { bytes: alloc::vec![kind + 1; 32] });
            assert_eq!(get(&mut ime, d, kind), ImeRes::Unset {});
        }
        assert_eq!(ime.run(t, ImeOp::Destroy { himc: a }), ImeRes::Ok {});
        assert_eq!(get(&mut ime, a, 0), ImeRes::Err { code: 6 });
    }
}
