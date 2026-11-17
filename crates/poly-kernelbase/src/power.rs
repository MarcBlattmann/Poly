//! Power requests (`PowerCreateRequest`, `PowerSetRequest`,
//! `PowerClearRequest`; sheets `kernel32.dll/PowerCreateRequest` and its
//! two pointer sheets; Learn's pages of the three, `winnt.h` for
//! `REASON_CONTEXT` and `POWER_REQUEST_TYPE`). Poly never sleeps on its
//! own, so a request changes nothing; what is kept is what the calls
//! answer: every request counts its sets per type, and only a request
//! handle is one.

use crate::error::{ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED};
use alloc::vec::Vec;

/// `POWER_REQUEST_CONTEXT_VERSION` (`winnt.h`).
pub const POWER_REQUEST_CONTEXT_VERSION: u32 = 0;
/// `POWER_REQUEST_CONTEXT_SIMPLE_STRING` (`winnt.h`).
pub const POWER_REQUEST_CONTEXT_SIMPLE_STRING: u32 = 1;
/// `POWER_REQUEST_CONTEXT_DETAILED_STRING` (`winnt.h`).
pub const POWER_REQUEST_CONTEXT_DETAILED_STRING: u32 = 2;
/// The `POWER_REQUEST_TYPE`s (`winnt.h`): display (0), system (1), away
/// mode (2), execution (3).
pub const POWER_REQUEST_TYPES: u32 = 4;

/// The parts of a `REASON_CONTEXT` the checks look at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
    pub version: u32,
    pub flags: u32,
    /// The simple string's length in units (`None` for a NULL pointer).
    pub simple_len: Option<usize>,
    /// The detailed form: whether a module is given, and the string count.
    pub detailed_module: bool,
    pub detailed_count: u32,
}

/// Whether `PowerCreateRequest` takes `c`: version 0, exactly one of the two
/// forms in the low two bits (the bits above are ignored), a non-empty simple
/// string, or a detailed form with a module. Anything else is
/// `ERROR_INVALID_PARAMETER`.
pub fn check_context(c: &Context) -> Result<(), u32> {
    if c.version != POWER_REQUEST_CONTEXT_VERSION {
        return Err(ERROR_INVALID_PARAMETER);
    }
    match c.flags & 3 {
        POWER_REQUEST_CONTEXT_SIMPLE_STRING if c.simple_len.is_some_and(|n| n > 0) => Ok(()),
        POWER_REQUEST_CONTEXT_DETAILED_STRING if c.detailed_module => Ok(()),
        _ => Err(ERROR_INVALID_PARAMETER),
    }
}

/// The live requests of a process, by handle value, with their counts.
#[derive(Debug, Default)]
pub struct Requests {
    live: Vec<(usize, [u32; POWER_REQUEST_TYPES as usize])>,
}

impl Requests {
    pub const fn new() -> Requests {
        Requests { live: Vec::new() }
    }

    /// A new request on handle `h`, with nothing set.
    pub fn add(&mut self, h: usize) {
        self.forget(h);
        self.live.push((h, [0; POWER_REQUEST_TYPES as usize]));
    }

    /// Handle `h` is closed: it is no request any more (what was set goes
    /// with it).
    pub fn forget(&mut self, h: usize) {
        self.live.retain(|(x, _)| *x != h);
    }

    /// Whether `h` is a request's handle.
    pub fn is_request(&self, h: usize) -> bool {
        self.live.iter().any(|(x, _)| *x == h)
    }

    fn counts(&mut self, h: usize, kind: u32) -> Result<&mut u32, u32> {
        let (_, counts) = self.live.iter_mut().find(|(x, _)| *x == h).ok_or(ERROR_INVALID_HANDLE)?;
        counts.get_mut(kind as usize).ok_or(ERROR_NOT_SUPPORTED)
    }

    /// `PowerSetRequest(h, kind)`: one more set of `kind`. A handle that is
    /// no request is `ERROR_INVALID_HANDLE`, a type above 3
    /// `ERROR_NOT_SUPPORTED`.
    pub fn set(&mut self, h: usize, kind: u32) -> Result<(), u32> {
        let n = self.counts(h, kind)?;
        *n = n.saturating_add(1);
        Ok(())
    }

    /// `PowerClearRequest(h, kind)`: undo one set of `kind`; with none left
    /// `ERROR_NOT_SUPPORTED`.
    pub fn clear(&mut self, h: usize, kind: u32) -> Result<(), u32> {
        let n = self.counts(h, kind)?;
        if *n == 0 {
            return Err(ERROR_NOT_SUPPORTED);
        }
        *n -= 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple(len: usize) -> Context {
        Context { version: 0, flags: 1, simple_len: Some(len), ..Default::default() }
    }

    #[test]
    fn contexts() {
        assert_eq!(check_context(&simple(11)), Ok(()));
        assert_eq!(check_context(&Context { flags: 0x8000_0001, ..simple(11) }), Ok(()));
        for bad in [
            Context { version: 1, ..simple(11) },
            Context { version: 0xFFFF_FFFF, ..simple(11) },
            Context { flags: 0, ..simple(11) },
            Context { flags: 2, ..simple(11) },
            Context { flags: 3, ..simple(11) },
            simple(0),
            Context { simple_len: None, ..simple(1) },
        ] {
            assert_eq!(check_context(&bad), Err(ERROR_INVALID_PARAMETER), "{bad:?}");
        }
        assert_eq!(check_context(&Context { flags: 2, detailed_module: true, ..Default::default() }), Ok(()));
    }

    #[test]
    fn counting() {
        let mut r = Requests::new();
        r.add(0x40);
        for kind in 0..4 {
            assert_eq!(r.set(0x40, kind), Ok(()));
            assert_eq!(r.clear(0x40, kind), Ok(()));
        }
        assert_eq!(r.set(0x40, 4), Err(ERROR_NOT_SUPPORTED));
        assert_eq!(r.set(0x40, u32::MAX), Err(ERROR_NOT_SUPPORTED));
        assert_eq!(r.clear(0x40, 4), Err(ERROR_NOT_SUPPORTED));
        // Two sets need two clears.
        r.set(0x40, 3).unwrap();
        r.set(0x40, 3).unwrap();
        assert_eq!(r.clear(0x40, 3), Ok(()));
        assert_eq!(r.clear(0x40, 3), Ok(()));
        assert_eq!(r.clear(0x40, 3), Err(ERROR_NOT_SUPPORTED));
        // Types and requests are independent.
        r.set(0x40, 1).unwrap();
        assert_eq!(r.clear(0x40, 3), Err(ERROR_NOT_SUPPORTED));
        r.add(0x44);
        assert_eq!(r.clear(0x44, 1), Err(ERROR_NOT_SUPPORTED));
        assert_eq!(r.clear(0x40, 1), Ok(()));
        // A closed handle is no request.
        r.forget(0x40);
        assert_eq!(r.set(0x40, 3), Err(ERROR_INVALID_HANDLE));
        assert_eq!(r.set(0x48, 3), Err(ERROR_INVALID_HANDLE));
        assert!(r.is_request(0x44));
    }
}
