//! Event Tracing for Windows when nobody listens (ADR 0155).
//!
//! Chrome registers about a dozen ETW providers in every process
//! (`EventRegister`, `RegisterTraceGuidsW`, `EventSetInformation`) and
//! writes events through them (`EventWrite*`, `TraceEvent`). Poly has no
//! trace sessions, so no provider is ever enabled: Learn documents what
//! every function does then. A registration succeeds and gives a handle,
//! `EventEnabled` and `EventProviderEnabled` say `FALSE`, a write returns
//! `ERROR_SUCCESS` without logging anything and the provider's enable
//! callback is never called (it is called only when a session enables
//! the provider).
//!
//! This crate holds what is not about the machine: the table of
//! registrations, the argument checks of each function and the activity-id
//! rules. `advapi32.dll` calls it.
//!
//! Sources (clean room): Learn's pages "EventRegister", "EventUnregister",
//! "EventSetInformation", "EventEnabled", "EventProviderEnabled",
//! "EventWrite", "EventWriteEx", "EventWriteString", "EventWriteTransfer",
//! "EventActivityIdControl", "RegisterTraceGuidsW", "UnregisterTraceGuids",
//! "TraceEvent", "GetTraceEnableFlags", "GetTraceEnableLevel",
//! "GetTraceLoggerHandle", "EVENT_DESCRIPTOR", "EVENT_DATA_DESCRIPTOR",
//! "EVENT_TRACE_HEADER", "WNODE_HEADER"; the constants of `evntprov.h`,
//! `evntrace.h`, `wmistr.h` and `winerror.h`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;

/// `ERROR_SUCCESS`, `ERROR_INVALID_HANDLE`, `ERROR_NOT_ENOUGH_MEMORY`,
/// `ERROR_INVALID_PARAMETER` (`winerror.h`).
pub const ERROR_SUCCESS: u32 = 0;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
/// `ERROR_ARITHMETIC_OVERFLOW` (`winerror.h`): too many registrations.
pub const ERROR_ARITHMETIC_OVERFLOW: u32 = 534;

/// `INVALID_HANDLE_VALUE` as a `TRACEHANDLE` (what `GetTraceLoggerHandle`
/// returns on failure).
pub const INVALID_TRACEHANDLE: u64 = u64::MAX;

/// Learn ("EventRegister"): at most this many registrations per process.
pub const MAX_REGISTRATIONS: usize = 1024;

/// `MAX_EVENT_DATA_DESCRIPTORS` (`evntprov.h`): the most
/// `EVENT_DATA_DESCRIPTOR`s one `EventWrite` takes.
pub const MAX_EVENT_DATA_DESCRIPTORS: u32 = 128;

/// `sizeof(EVENT_TRACE_HEADER)` (`evntrace.h`): the smallest valid `Size`
/// of a header given to `TraceEvent`.
pub const EVENT_TRACE_HEADER_SIZE: u16 = 48;

/// The `EVENT_INFO_CLASS` values of `evntprov.h` (`EventSetInformation`).
pub mod info_class {
    pub const EVENT_PROVIDER_BINARY_TRACK_INFO: u32 = 0;
    pub const EVENT_PROVIDER_SET_RESERVED1: u32 = 1;
    pub const EVENT_PROVIDER_SET_TRAITS: u32 = 2;
    pub const EVENT_PROVIDER_USE_DESCRIPTOR_TYPE: u32 = 3;
    pub const EVENT_INFO_MAX: u32 = 4;
}

/// The `EventActivityIdControl` controls (`evntprov.h`).
pub mod activity {
    pub const EVENT_ACTIVITY_CTRL_GET_ID: u32 = 1;
    pub const EVENT_ACTIVITY_CTRL_SET_ID: u32 = 2;
    pub const EVENT_ACTIVITY_CTRL_CREATE_ID: u32 = 3;
    pub const EVENT_ACTIVITY_CTRL_GET_SET_ID: u32 = 4;
    pub const EVENT_ACTIVITY_CTRL_CREATE_SET_ID: u32 = 5;
}

/// The two kinds of registration: the manifest/TraceLogging API
/// (`EventRegister`) and the classic one (`RegisterTraceGuidsW`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Modern,
    Classic,
}

/// One registered provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Provider {
    pub handle: u64,
    pub guid: [u8; 16],
    pub kind: Kind,
    /// The enable callback and its context, as the caller passed them
    /// (never called: nothing enables a provider).
    pub callback: usize,
    pub context: usize,
}

/// The providers a process registered.
#[derive(Debug, Default)]
pub struct Registrations {
    list: Vec<Provider>,
    /// The last handle's low part; handles are never reused.
    next: u64,
}

/// Handles are non-zero, not -1 and recognisable in a log.
const HANDLE_BASE: u64 = 0x0E7C_0000_0000;

impl Registrations {
    pub const fn new() -> Registrations {
        Registrations { list: Vec::new(), next: 0 }
    }

    /// `EventRegister` / `RegisterTraceGuidsW`: the handle of the new registration.
    pub fn register(&mut self, guid: [u8; 16], kind: Kind, callback: usize, context: usize) -> Result<u64, u32> {
        if self.list.len() >= MAX_REGISTRATIONS {
            return Err(ERROR_ARITHMETIC_OVERFLOW);
        }
        self.list.try_reserve(1).map_err(|_| ERROR_NOT_ENOUGH_MEMORY)?;
        self.next += 1;
        let handle = HANDLE_BASE + self.next;
        self.list.push(Provider { handle, guid, kind, callback, context });
        Ok(handle)
    }

    /// `EventUnregister` / `UnregisterTraceGuids`: `ERROR_INVALID_HANDLE` for
    /// a handle that is not (or no longer) registered.
    pub fn unregister(&mut self, handle: u64) -> Result<(), u32> {
        match self.list.iter().position(|p| p.handle == handle) {
            Some(i) => {
                self.list.remove(i);
                Ok(())
            }
            None => Err(ERROR_INVALID_HANDLE),
        }
    }

    pub fn get(&self, handle: u64) -> Option<&Provider> {
        self.list.iter().find(|p| p.handle == handle)
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// `EventSetInformation`: the classes of `EVENT_INFO_CLASS` are accepted
    /// for a registered handle (the data is traits or tracking information
    /// for sessions that do not exist); anything else is
    /// `ERROR_INVALID_PARAMETER`; an unknown handle `ERROR_INVALID_HANDLE`.
    pub fn set_information(&self, handle: u64, class: u32, info_present: bool, length: u32) -> u32 {
        if self.get(handle).is_none() {
            return ERROR_INVALID_HANDLE;
        }
        if class >= info_class::EVENT_INFO_MAX {
            return ERROR_INVALID_PARAMETER;
        }
        if length != 0 && !info_present {
            return ERROR_INVALID_PARAMETER;
        }
        ERROR_SUCCESS
    }

    /// `EventWrite*`: a registered handle and at most 128 data descriptors
    /// give `ERROR_SUCCESS` (nothing is logged: no session is enabled).
    pub fn write(&self, handle: u64, user_data_count: u32, user_data_present: bool) -> u32 {
        if self.get(handle).is_none() {
            return ERROR_INVALID_HANDLE;
        }
        if user_data_count > MAX_EVENT_DATA_DESCRIPTORS || (user_data_count > 0 && !user_data_present) {
            return ERROR_INVALID_PARAMETER;
        }
        ERROR_SUCCESS
    }

    /// `EventEnabled` / `EventProviderEnabled`: never enabled.
    pub fn enabled(&self, _handle: u64) -> bool {
        false
    }
}

/// `TraceEvent(TraceHandle, EVENT_TRACE_HEADER*)`: the header must exist and
/// its `Size` (first `USHORT`) cover the structure; with no session the
/// result is `ERROR_SUCCESS`.
pub fn trace_event(header_size: Option<u16>) -> u32 {
    match header_size {
        Some(s) if s >= EVENT_TRACE_HEADER_SIZE => ERROR_SUCCESS,
        _ => ERROR_INVALID_PARAMETER,
    }
}

/// `WNODE_HEADER` (`wmistr.h`): `BufferSize` ULONG, `ProviderId` ULONG, then
/// `HistoricalContext` ULONG64 at offset 8: the logger handle
/// `GetTraceLoggerHandle` returns.
pub const WNODE_HISTORICAL_CONTEXT: usize = 8;

/// `GetTraceLoggerHandle` for the bytes of a `WNODE_HEADER` (an error: the
/// call failed and returns `INVALID_HANDLE_VALUE`).
pub fn trace_logger_handle(wnode: Option<&[u8]>) -> Result<u64, u32> {
    let b = wnode.ok_or(ERROR_INVALID_HANDLE)?;
    let at = b.get(WNODE_HISTORICAL_CONTEXT..WNODE_HISTORICAL_CONTEXT + 8).ok_or(ERROR_INVALID_HANDLE)?;
    let mut v = [0u8; 8];
    v.copy_from_slice(at);
    Ok(u64::from_le_bytes(v))
}

/// A GUID made from 16 random bytes: version 4, variant 1 (RFC 4122), as
/// `EventActivityIdControl(EVENT_ACTIVITY_CTRL_CREATE_ID)` gives.
pub fn new_activity_id(mut random: [u8; 16]) -> [u8; 16] {
    // Data3's high nibble (bytes 6..8, little-endian: byte 7) holds the version.
    random[7] = (random[7] & 0x0F) | 0x40;
    // Data4[0]'s top bits (byte 8) hold the variant.
    random[8] = (random[8] & 0x3F) | 0x80;
    random
}

/// `EventActivityIdControl`: `control` and the id passed in `io`; `current`
/// is the thread's id (all zero at first). Returns the Win32 error; `io` is
/// read or written as the control says.
pub fn activity_id_control(control: u32, io: Option<&mut [u8; 16]>, current: &mut [u8; 16], random: [u8; 16]) -> u32 {
    use activity::*;
    let Some(io) = io else { return ERROR_INVALID_PARAMETER };
    match control {
        EVENT_ACTIVITY_CTRL_GET_ID => *io = *current,
        EVENT_ACTIVITY_CTRL_SET_ID => *current = *io,
        EVENT_ACTIVITY_CTRL_CREATE_ID => *io = new_activity_id(random),
        EVENT_ACTIVITY_CTRL_GET_SET_ID => core::mem::swap(io, current),
        EVENT_ACTIVITY_CTRL_CREATE_SET_ID => {
            *io = *current;
            *current = new_activity_id(random);
        }
        _ => return ERROR_INVALID_PARAMETER,
    }
    ERROR_SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: [u8; 16] = [1; 16];

    #[test]
    fn register_gives_distinct_nonzero_handles_and_unregister_removes() {
        let mut r = Registrations::new();
        let a = r.register(G, Kind::Modern, 0x1000, 7).unwrap();
        let b = r.register(G, Kind::Classic, 0, 0).unwrap();
        assert_ne!(a, b);
        assert!(a != 0 && a != u64::MAX && b != 0 && b != u64::MAX);
        assert_eq!(r.len(), 2);
        assert_eq!(r.get(a).unwrap().context, 7);
        assert_eq!(r.unregister(a), Ok(()));
        assert_eq!(r.unregister(a), Err(ERROR_INVALID_HANDLE));
        assert_eq!(r.len(), 1);
        // A handle is never reused.
        let c = r.register(G, Kind::Modern, 0, 0).unwrap();
        assert!(c != a && c != b);
    }

    #[test]
    fn too_many_registrations_overflow() {
        let mut r = Registrations::new();
        for _ in 0..MAX_REGISTRATIONS {
            r.register(G, Kind::Modern, 0, 0).unwrap();
        }
        assert_eq!(r.register(G, Kind::Modern, 0, 0), Err(ERROR_ARITHMETIC_OVERFLOW));
    }

    #[test]
    fn writes_succeed_quietly_for_registered_handles_only() {
        let mut r = Registrations::new();
        let h = r.register(G, Kind::Modern, 0, 0).unwrap();
        assert_eq!(r.write(h, 0, false), ERROR_SUCCESS);
        assert_eq!(r.write(h, 3, true), ERROR_SUCCESS);
        assert_eq!(r.write(h, 129, true), ERROR_INVALID_PARAMETER);
        assert_eq!(r.write(h, 1, false), ERROR_INVALID_PARAMETER);
        assert_eq!(r.write(h + 99, 0, false), ERROR_INVALID_HANDLE);
        assert!(!r.enabled(h));
    }

    #[test]
    fn set_information_checks_handle_class_and_buffer() {
        let mut r = Registrations::new();
        let h = r.register(G, Kind::Modern, 0, 0).unwrap();
        assert_eq!(r.set_information(h, info_class::EVENT_PROVIDER_SET_TRAITS, true, 40), ERROR_SUCCESS);
        assert_eq!(r.set_information(h, info_class::EVENT_PROVIDER_SET_TRAITS, false, 0), ERROR_SUCCESS);
        assert_eq!(r.set_information(h, info_class::EVENT_PROVIDER_SET_TRAITS, false, 4), ERROR_INVALID_PARAMETER);
        assert_eq!(r.set_information(h, 9, true, 4), ERROR_INVALID_PARAMETER);
        assert_eq!(r.set_information(h + 1, 2, true, 4), ERROR_INVALID_HANDLE);
    }

    #[test]
    fn trace_event_needs_a_full_header() {
        assert_eq!(trace_event(Some(48)), ERROR_SUCCESS);
        assert_eq!(trace_event(Some(200)), ERROR_SUCCESS);
        assert_eq!(trace_event(Some(47)), ERROR_INVALID_PARAMETER);
        assert_eq!(trace_event(None), ERROR_INVALID_PARAMETER);
    }

    #[test]
    fn logger_handle_is_the_wnodes_historical_context() {
        let mut w = [0u8; 48];
        w[8..16].copy_from_slice(&0x1234_5678_9ABCu64.to_le_bytes());
        assert_eq!(trace_logger_handle(Some(&w)), Ok(0x1234_5678_9ABC));
        assert_eq!(trace_logger_handle(Some(&w[..12])), Err(ERROR_INVALID_HANDLE));
        assert_eq!(trace_logger_handle(None), Err(ERROR_INVALID_HANDLE));
    }

    #[test]
    fn activity_ids_follow_the_controls() {
        let mut cur = [0u8; 16];
        let rnd = [0xFF; 16];
        let mut io = [9u8; 16];
        assert_eq!(activity_id_control(activity::EVENT_ACTIVITY_CTRL_GET_ID, Some(&mut io), &mut cur, rnd), 0);
        assert_eq!(io, [0; 16]);
        let mut id = [5u8; 16];
        assert_eq!(activity_id_control(activity::EVENT_ACTIVITY_CTRL_SET_ID, Some(&mut id), &mut cur, rnd), 0);
        assert_eq!(cur, [5; 16]);
        // CREATE_ID leaves the thread's id alone and makes a version-4 GUID.
        let mut made = [0u8; 16];
        assert_eq!(activity_id_control(activity::EVENT_ACTIVITY_CTRL_CREATE_ID, Some(&mut made), &mut cur, rnd), 0);
        assert_eq!(cur, [5; 16]);
        assert_eq!(made[7] >> 4, 4);
        assert_eq!(made[8] >> 6, 2);
        // GET_SET swaps; CREATE_SET returns the old id and installs a new one.
        let mut x = [7u8; 16];
        assert_eq!(activity_id_control(activity::EVENT_ACTIVITY_CTRL_GET_SET_ID, Some(&mut x), &mut cur, rnd), 0);
        assert_eq!((x, cur), ([5; 16], [7; 16]));
        let mut y = [0u8; 16];
        assert_eq!(activity_id_control(activity::EVENT_ACTIVITY_CTRL_CREATE_SET_ID, Some(&mut y), &mut cur, rnd), 0);
        assert_eq!(y, [7; 16]);
        assert_eq!(cur[7] >> 4, 4);
        assert_eq!(activity_id_control(99, Some(&mut y), &mut cur, rnd), ERROR_INVALID_PARAMETER);
        assert_eq!(activity_id_control(1, None, &mut cur, rnd), ERROR_INVALID_PARAMETER);
    }
}
