//! COM's process-wide options (`CLSID_GlobalOptions`, `IGlobalOptions::Set` and `Query`): which
//! values each property takes, which can be changed back, and what a read gives.
//!
//! Sheets `CLSID_GlobalOptions`, `IGlobalOptions.Set`, `IGlobalOptions.Query`; the property numbers
//! are `GLOBALOPT_PROPERTIES` (`objidl.h`, Learn "GLOBALOPT_PROPERTIES enumeration").

use crate::hresult::{E_FAIL, E_INVALIDARG, Hresult, RPC_E_TOO_LATE};

/// `COMGLB_EXCEPTION_HANDLING`.
pub const EXCEPTION_HANDLING: u32 = 1;
/// `COMGLB_APPID`.
pub const APPID: u32 = 2;
/// `COMGLB_RPC_THREADPOOL_SETTING`.
pub const RPC_THREADPOOL_SETTING: u32 = 3;
/// `COMGLB_RO_SETTINGS`.
pub const RO_SETTINGS: u32 = 4;
/// `COMGLB_UNMARSHALING_POLICY`.
pub const UNMARSHALING_POLICY: u32 = 5;
/// `COMGLB_PROPERTIES_RESERVED1..3`.
pub const RESERVED1: u32 = 6;
pub const RESERVED2: u32 = 7;
pub const RESERVED3: u32 = 8;

/// `COMGLB_EXCEPTION_HANDLE`: COM catches exceptions in the calls it serves (the default).
pub const EXCEPTION_HANDLE: usize = 0;
/// `COMGLB_EXCEPTION_DONOT_HANDLE_FATAL`: access violations are not caught.
pub const EXCEPTION_DONOT_HANDLE_FATAL: usize = 1;
/// `COMGLB_EXCEPTION_DONOT_HANDLE_ANY`: nothing is caught.
pub const EXCEPTION_DONOT_HANDLE_ANY: usize = 2;

/// What `Set(COMGLB_RPC_THREADPOOL_SETTING, 1)` gives once the process has used RPC (sheet
/// `IGlobalOptions.Set`, rule `rpc_threadpool`: the number the oracle returned).
pub const THREADPOOL_TOO_LATE: Hresult = 0x8001_06E4_u32 as i32;

/// The `COMGLB_RO_SETTINGS` bits that stay once set: changing the value again is "too late".
const RO_STICKY: usize = 0x8 | 0x10 | 0x20 | 0x40;
/// The bit a `Set` cannot clear (it is kept beside the new value).
const RO_KEPT: usize = 0x100;
/// The highest bit `COMGLB_RO_SETTINGS` takes.
const RO_MASK: usize = 0x3FF;

/// The options of one process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    exception_handling: usize,
    ro_settings: usize,
    unmarshaling_policy: usize,
    reserved3: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self::new()
    }
}

impl Options {
    /// A new process's values (sheet `IGlobalOptions.Query`, rule `defaults`).
    pub const fn new() -> Self {
        Self { exception_handling: EXCEPTION_HANDLE, ro_settings: 0, unmarshaling_policy: 2, reserved3: 0 }
    }

    /// The exception-handling setting (`COMGLB_EXCEPTION_*`).
    pub fn exception_handling(&self) -> usize {
        self.exception_handling
    }

    /// `IGlobalOptions::Set(property, value)`. `rpc_used`: the process has marshalled, tried a
    /// local server or set its security (the thread-pool setting is then fixed). A failure leaves
    /// every value.
    pub fn set(&mut self, property: u32, value: usize, rpc_used: bool) -> Result<(), Hresult> {
        match property {
            EXCEPTION_HANDLING => match value {
                // The default cannot be asked for again, also where nothing was set.
                EXCEPTION_HANDLE => Err(E_FAIL),
                EXCEPTION_DONOT_HANDLE_FATAL | EXCEPTION_DONOT_HANDLE_ANY => {
                    self.exception_handling = value;
                    Ok(())
                }
                _ => Err(E_INVALIDARG),
            },
            APPID => Err(E_FAIL),
            RPC_THREADPOOL_SETTING => match value {
                1 if rpc_used => Err(THREADPOOL_TOO_LATE),
                1 => Ok(()),
                _ => Err(E_INVALIDARG),
            },
            RO_SETTINGS => {
                if value & !RO_MASK != 0 || matches!(value & 7, 6 | 7) {
                    return Err(E_INVALIDARG);
                }
                if self.ro_settings & RO_STICKY != 0 && value != self.ro_settings {
                    return Err(RPC_E_TOO_LATE);
                }
                self.ro_settings = value | (self.ro_settings & RO_KEPT);
                Ok(())
            }
            UNMARSHALING_POLICY => match value {
                0..=2 => {
                    self.unmarshaling_policy = value;
                    Ok(())
                }
                _ => Err(E_INVALIDARG),
            },
            RESERVED3 => {
                if value == 0 || value == 1 || value as u32 == u32::MAX {
                    self.reserved3 = value;
                    Ok(())
                } else {
                    Err(E_INVALIDARG)
                }
            }
            // The oracle crashed in Set of the two other reserved properties (not specified); here
            // they are refused.
            _ => Err(E_INVALIDARG),
        }
    }

    /// `IGlobalOptions::Query(property)`. `reserved2`: what the second reserved property reads (a
    /// pointer-sized value the caller picks).
    pub fn query(&self, property: u32, reserved2: usize) -> Result<usize, Hresult> {
        match property {
            EXCEPTION_HANDLING => Ok(self.exception_handling),
            APPID => Err(E_FAIL),
            RPC_THREADPOOL_SETTING => Ok(1),
            RO_SETTINGS => Ok(self.ro_settings),
            UNMARSHALING_POLICY => Ok(self.unmarshaling_policy),
            RESERVED1 => Ok(0),
            RESERVED2 => Ok(reserved2),
            RESERVED3 => Ok(self.reserved3),
            _ => Err(E_INVALIDARG),
        }
    }
}

/// Whether an exception with `code` raised in a call COM serves is caught (the caller gets
/// `RPC_E_SERVERFAULT`) under the exception-handling setting `setting` (sheet
/// `GlobalOptions.ExceptionHandling`): by default every one, with "do not handle fatal" all but an
/// access violation, with "do not handle any" none.
pub fn catches(setting: usize, code: u32) -> bool {
    /// `STATUS_ACCESS_VIOLATION` (`ntstatus.h`).
    const ACCESS_VIOLATION: u32 = 0xC000_0005;
    match setting {
        EXCEPTION_DONOT_HANDLE_ANY => false,
        EXCEPTION_DONOT_HANDLE_FATAL => code != ACCESS_VIOLATION,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    fn run(property: u32, values: &[usize]) -> (Vec<Result<(), Hresult>>, Result<usize, Hresult>) {
        let mut o = Options::new();
        let r = values.iter().map(|&v| o.set(property, v, false)).collect();
        (r, o.query(property, 0x7FF0_0000))
    }

    #[test]
    fn defaults() {
        let o = Options::new();
        let q: Vec<_> = (0..=9).map(|p| o.query(p, 0x7FF0_0000)).collect();
        assert_eq!(
            q,
            [
                Err(E_INVALIDARG),
                Ok(0),
                Err(E_FAIL),
                Ok(1),
                Ok(0),
                Ok(2),
                Ok(0),
                Ok(0x7FF0_0000),
                Ok(0),
                Err(E_INVALIDARG)
            ]
        );
        assert_eq!(o.query(0xFFFF_FFFF, 0), Err(E_INVALIDARG));
    }

    #[test]
    fn exception_handling_cannot_go_back() {
        assert_eq!(run(1, &[0]), (vec![Err(E_FAIL)], Ok(0)));
        assert_eq!(run(1, &[1, 2, 1, 2]), (vec![Ok(()); 4], Ok(2)));
        assert_eq!(run(1, &[2, 0]), (vec![Ok(()), Err(E_FAIL)], Ok(2)));
        assert_eq!(run(1, &[2, 3, 2]), (vec![Ok(()), Err(E_INVALIDARG), Ok(())], Ok(2)));
        assert_eq!(run(1, &[0xFFFF_FFFF]), (vec![Err(E_INVALIDARG)], Ok(0)));
    }

    #[test]
    fn thread_pool_and_policy() {
        assert_eq!(run(3, &[0, 1, 2]), (vec![Err(E_INVALIDARG), Ok(()), Err(E_INVALIDARG)], Ok(1)));
        let mut o = Options::new();
        assert_eq!(o.set(3, 1, true), Err(THREADPOOL_TOO_LATE));
        assert_eq!(o.set(3, 0, true), Err(E_INVALIDARG));
        assert_eq!(run(5, &[2, 1, 0]), (vec![Ok(()); 3], Ok(0)));
        assert_eq!(run(5, &[3, 1]), (vec![Err(E_INVALIDARG), Ok(())], Ok(1)));
    }

    #[test]
    fn ro_settings() {
        for v in [0, 1, 2, 3, 4, 5, 8, 9, 0x10, 0x20, 0x40, 0x80, 0x100, 0x180, 0x200] {
            assert_eq!(run(4, &[v]), (vec![Ok(())], Ok(v)), "{v:#x}");
        }
        for v in [6, 7, 0x400, 0x800, 0x1000, 0x8000, 0x10000, 0xFFFF, 0xFFFF_FFFF] {
            assert_eq!(run(4, &[v]), (vec![Err(E_INVALIDARG)], Ok(0)), "{v:#x}");
        }
        assert_eq!(run(4, &[3, 5, 0]), (vec![Ok(()); 3], Ok(0)));
        assert_eq!(run(4, &[0x80, 0]), (vec![Ok(()); 2], Ok(0)));
        assert_eq!(run(4, &[8, 0]), (vec![Ok(()), Err(RPC_E_TOO_LATE)], Ok(8)));
        assert_eq!(run(4, &[0x10, 0x20]), (vec![Ok(()), Err(RPC_E_TOO_LATE)], Ok(0x10)));
        assert_eq!(run(4, &[0x100, 0]), (vec![Ok(()); 2], Ok(0x100)));
        assert_eq!(run(4, &[0x100, 0x200]), (vec![Ok(()); 2], Ok(0x300)));
    }

    #[test]
    fn reserved_and_appid() {
        assert_eq!(run(8, &[1, 0]), (vec![Ok(()); 2], Ok(0)));
        assert_eq!(run(8, &[2]), (vec![Err(E_INVALIDARG)], Ok(0)));
        assert_eq!(run(8, &[0xFFFF_FFFF]), (vec![Ok(())], Ok(0xFFFF_FFFF)));
        assert_eq!(run(2, &[0]), (vec![Err(E_FAIL)], Err(E_FAIL)));
        assert_eq!(run(9, &[1]), (vec![Err(E_INVALIDARG)], Err(E_INVALIDARG)));
    }

    #[test]
    fn what_is_caught() {
        assert!(catches(0, 0xC000_0005) && catches(0, 0xE000_0001));
        assert!(!catches(1, 0xC000_0005) && catches(1, 0xC000_0094));
        assert!(!catches(2, 0xE000_0001));
    }
}
