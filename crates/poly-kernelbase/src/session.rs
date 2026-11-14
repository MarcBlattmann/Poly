//! What the small session and power DLLs answer (ADR 0155, step 9): `wtsapi32.dll`
//! (`WTSQuerySessionInformationW`), `powrprof.dll` behind the API set `api-ms-win-power-base`
//! (`CallNtPowerInformation`) and `comctl32.dll` (`InitCommonControlsEx`). Chrome's browser
//! delay-loads all three when it starts; a DLL that does not load ends it.
//!
//! Poly has one session, at the console, with one user, on a machine that is on mains power.
//!
//! Sources, Microsoft Learn: "WTSQuerySessionInformationW function (wtsapi32.h)",
//! "WTS_INFO_CLASS", "WTS_CONNECTSTATE_CLASS"; "CallNtPowerInformation function
//! (powerbase.h)", "POWER_INFORMATION_LEVEL", "PROCESSOR_POWER_INFORMATION structure",
//! "SYSTEM_BATTERY_STATE structure (winnt.h)", "PowerDeterminePlatformRoleEx";
//! "InitCommonControlsEx function (commctrl.h)" and "INITCOMMONCONTROLSEX". Doc-derived, not
//! observed on Windows.

use crate::error::ERROR_INVALID_PARAMETER;
use alloc::vec::Vec;

// ----- wtsapi32 -------------------------------------------------------------------------------

/// The last error for a session that does not exist (the sheet: 1733, as observed).
pub const WTS_UNKNOWN_SESSION: u32 = 1733;
/// `WTS_CURRENT_SESSION` (`wtsapi32.h`).
pub const WTS_CURRENT_SESSION: u32 = 0xFFFF_FFFF;
/// `WTS_INFO_CLASS` values (`wtsapi32.h`).
pub const WTS_INITIAL_PROGRAM: u32 = 0;
pub const WTS_APPLICATION_NAME: u32 = 1;
pub const WTS_WORKING_DIRECTORY: u32 = 2;
pub const WTS_SESSION_ID: u32 = 4;
pub const WTS_USER_NAME: u32 = 5;
pub const WTS_WIN_STATION_NAME: u32 = 6;
pub const WTS_DOMAIN_NAME: u32 = 7;
pub const WTS_CONNECT_STATE: u32 = 8;
pub const WTS_CLIENT_NAME: u32 = 10;
pub const WTS_CLIENT_PROTOCOL_TYPE: u32 = 16;
/// `WTSSessionInfoEx`: a `WTSINFOEXW` (the class Chrome asks, the sheet's `wtsq_chrome`).
pub const WTS_SESSION_INFO_EX: u32 = 25;
/// `sizeof(WTSINFOEXW)` on x64 (`wtsapi32.h`: `Level`, then `WTSINFOEX_LEVEL1_W`).
pub const WTSINFOEX_SIZE: usize = 232;
/// `WTS_SESSIONSTATE_UNLOCK` (`wtsapi32.h`): the session is not locked.
pub const WTS_SESSIONSTATE_UNLOCK: u32 = 1;
pub const WTS_IS_REMOTE_SESSION: u32 = 29;
/// `WTSActive` (`WTS_CONNECTSTATE_CLASS`): "A user is logged on to the WinStation".
pub const WTS_ACTIVE: u32 = 0;
/// `NOTIFY_FOR_ALL_SESSIONS` (`wtsapi32.h`); `NOTIFY_FOR_THIS_SESSION` is 0.
pub const NOTIFY_FOR_ALL_SESSIONS: u32 = 1;

/// The one session Poly has, as `WTSQuerySessionInformationW` describes it.
#[derive(Clone, Copy, Debug)]
pub struct Session<'a> {
    pub id: u32,
    pub user: &'a [u16],
    /// The computer's name: the "domain" of a local account.
    pub computer: &'a [u16],
    /// The current time as a `FILETIME` (for `WTSINFOEX_LEVEL1_W.CurrentTime`).
    pub now: u64,
}

fn text(s: &[u16]) -> Vec<u8> {
    s.iter().chain(core::iter::once(&0)).flat_map(|c| c.to_le_bytes()).collect()
}

/// `WTSQuerySessionInformationW(server, session, class)`: the bytes the caller gets (strings
/// with their NUL). `session` must be `WTS_CURRENT_SESSION` or this session's id
/// (`ERROR_INVALID_PARAMETER` otherwise: Poly has no other session); a class Poly has no answer
/// for is `ERROR_INVALID_PARAMETER` too (the sheet: class 100 is error 87, the out pointer
/// untouched).
///
/// The session is the console's: the WinStation is `Console`, the protocol type 0 ("the console
/// session", Learn), not remote, no client name, no initial program.
pub fn wts_query(s: &Session<'_>, session: u32, class: u32) -> Result<Vec<u8>, u32> {
    // The sheet's rule `unknown_session`: a session that does not exist is error 1733 (as
    // observed), except for `WTSClientProtocolType`, which answers for it all the same.
    if session != WTS_CURRENT_SESSION && session != s.id && class != WTS_CLIENT_PROTOCOL_TYPE {
        return Err(WTS_UNKNOWN_SESSION);
    }
    let console: Vec<u16> = "Console".encode_utf16().collect();
    Ok(match class {
        WTS_INITIAL_PROGRAM | WTS_APPLICATION_NAME | WTS_WORKING_DIRECTORY | WTS_CLIENT_NAME => text(&[]),
        WTS_SESSION_ID => s.id.to_le_bytes().to_vec(),
        WTS_USER_NAME => text(s.user),
        WTS_WIN_STATION_NAME => text(&console),
        WTS_DOMAIN_NAME => text(s.computer),
        WTS_CONNECT_STATE => WTS_ACTIVE.to_le_bytes().to_vec(),
        WTS_CLIENT_PROTOCOL_TYPE => 0u16.to_le_bytes().to_vec(),
        WTS_IS_REMOTE_SESSION => alloc::vec![0u8],
        WTS_SESSION_INFO_EX => {
            // WTSINFOEXW: Level 1 at 0; WTSINFOEX_LEVEL1_W at 8: SessionId, SessionState,
            // SessionFlags, WinStationName[33] at 20, UserName[21] at 86, DomainName[18] at
            // 128, the five times at 168 (CurrentTime the last, at 200), six counters at 208.
            let mut b = alloc::vec![0u8; WTSINFOEX_SIZE];
            b[0..4].copy_from_slice(&1u32.to_le_bytes());
            b[8..12].copy_from_slice(&s.id.to_le_bytes());
            b[12..16].copy_from_slice(&WTS_ACTIVE.to_le_bytes());
            b[16..20].copy_from_slice(&WTS_SESSIONSTATE_UNLOCK.to_le_bytes());
            for (at, text, room) in [(20usize, &console[..], 32usize), (86, s.user, 20), (128, s.computer, 17)] {
                for (i, c) in text.iter().take(room).enumerate() {
                    b[at + 2 * i..at + 2 * i + 2].copy_from_slice(&c.to_le_bytes());
                }
            }
            b[200..208].copy_from_slice(&s.now.to_le_bytes());
            b
        }
        _ => return Err(ERROR_INVALID_PARAMETER),
    })
}

/// `WTSRegisterSessionNotification(hWnd, dwFlags)`: the flags are one of the two values; the
/// window must be given. Poly's session never changes state, so a registered window never gets
/// `WM_WTSSESSION_CHANGE`.
pub fn wts_register(hwnd: usize, flags: u32) -> Result<(), u32> {
    if hwnd == 0 || flags > NOTIFY_FOR_ALL_SESSIONS { Err(ERROR_INVALID_PARAMETER) } else { Ok(()) }
}

// ----- powrprof -------------------------------------------------------------------------------

/// `POWER_INFORMATION_LEVEL` values (`winnt.h`).
pub const SYSTEM_POWER_CAPABILITIES: u32 = 4;
/// `sizeof(SYSTEM_POWER_CAPABILITIES)` (`winnt.h`; the sheet: 76 bytes).
pub const SYSTEM_POWER_CAPABILITIES_SIZE: usize = 76;
pub const SYSTEM_BATTERY_STATE: u32 = 5;
/// `SystemExecutionState`: a `ULONG` of `ES_*` bits (the sheet's `cnpi_levels`).
pub const SYSTEM_EXECUTION_STATE: u32 = 16;
pub const PROCESSOR_INFORMATION: u32 = 11;
/// `STATUS_SUCCESS`, `STATUS_BUFFER_TOO_SMALL`, `STATUS_INVALID_PARAMETER`,
/// `STATUS_NOT_SUPPORTED` (`ntstatus.h`).
pub const STATUS_SUCCESS: u32 = 0;
pub const STATUS_BUFFER_TOO_SMALL: u32 = 0xC000_0023;
pub const STATUS_INVALID_PARAMETER: u32 = 0xC000_000D;
pub const STATUS_NOT_SUPPORTED: u32 = 0xC000_00BB;
/// `sizeof(PROCESSOR_POWER_INFORMATION)`: six `ULONG`s.
pub const PROCESSOR_POWER_INFORMATION_SIZE: usize = 24;
/// `sizeof(SYSTEM_BATTERY_STATE)` (`winnt.h`).
pub const SYSTEM_BATTERY_STATE_SIZE: usize = 32;
/// The processor speed Poly reports, in MHz (the nominal 3 GHz of `procinfo::cycle_time`).
pub const NOMINAL_MHZ: u32 = 3000;
/// `PlatformRoleDesktop` (`POWER_PLATFORM_ROLE`, `winnt.h`).
pub const PLATFORM_ROLE_DESKTOP: u32 = 1;

/// `CallNtPowerInformation(level, ..., output, output_size)`: the bytes for the output buffer,
/// or the `NTSTATUS`.
///
/// * `ProcessorInformation`: one `PROCESSOR_POWER_INFORMATION` per processor (number, maximum
///   and current MHz, MHz limit, maximum and current idle state); Learn: the buffer must hold
///   all of them, else `STATUS_BUFFER_TOO_SMALL`.
/// * `SystemBatteryState`: a machine on mains power without a battery (`AcOnLine` TRUE, the
///   rest zero).
/// * Any other level: `STATUS_NOT_SUPPORTED`.
pub fn power_information(level: u32, processors: u32, output_size: usize) -> Result<Vec<u8>, u32> {
    match level {
        PROCESSOR_INFORMATION => {
            let n = processors.max(1);
            // The sheet's test: no output bytes at all is STATUS_INVALID_PARAMETER.
            if output_size == 0 {
                return Err(STATUS_INVALID_PARAMETER);
            }
            if output_size < n as usize * PROCESSOR_POWER_INFORMATION_SIZE {
                return Err(STATUS_BUFFER_TOO_SMALL);
            }
            let mut out = Vec::with_capacity(n as usize * PROCESSOR_POWER_INFORMATION_SIZE);
            for number in 0..n {
                for v in [number, NOMINAL_MHZ, NOMINAL_MHZ, NOMINAL_MHZ, 0, 0] {
                    out.extend(v.to_le_bytes());
                }
            }
            Ok(out)
        }
        SYSTEM_BATTERY_STATE => {
            if output_size < SYSTEM_BATTERY_STATE_SIZE {
                return Err(STATUS_BUFFER_TOO_SMALL);
            }
            let mut out = alloc::vec![0u8; SYSTEM_BATTERY_STATE_SIZE];
            out[0] = 1;
            Ok(out)
        }
        SYSTEM_POWER_CAPABILITIES => {
            if output_size < SYSTEM_POWER_CAPABILITIES_SIZE {
                return Err(STATUS_BUFFER_TOO_SMALL);
            }
            // SYSTEM_POWER_CAPABILITIES (winnt.h) of a desktop: a power button
            // (`PowerButtonPresent`, byte 0) and soft off (`SystemS5`, byte 7); no sleep states,
            // no lid, no batteries.
            let mut out = alloc::vec![0u8; SYSTEM_POWER_CAPABILITIES_SIZE];
            out[0] = 1;
            out[7] = 1;
            Ok(out)
        }
        SYSTEM_EXECUTION_STATE => {
            if output_size < 4 {
                return Err(STATUS_BUFFER_TOO_SMALL);
            }
            // No thread asks the system to stay awake: no `ES_*` bit.
            Ok(alloc::vec![0u8; 4])
        }
        _ => Err(STATUS_NOT_SUPPORTED),
    }
}

// ----- comctl32 -------------------------------------------------------------------------------

/// `sizeof(INITCOMMONCONTROLSEX)`: `dwSize` and `dwICC`.
pub const INITCOMMONCONTROLSEX_SIZE: u32 = 8;

/// `InitCommonControlsEx(picce)`: Learn: "Returns TRUE if successful, or FALSE otherwise";
/// `dwSize` is "the size of the structure, in bytes". Poly accepts a structure of that size:
/// the control classes themselves are not there yet, so a program that then creates a common
/// control gets no window (`CreateWindowExW` fails for the unknown class).
pub fn init_common_controls(size: u32) -> bool {
    size == INITCOMMONCONTROLSEX_SIZE
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::vec;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn the_console_session_answers_the_classes_a_browser_asks() {
        let (user, computer) = (w("poly"), w("POLY-PC"));
        let s = Session { id: 1, user: &user, computer: &computer, now: 0x01DC_0000_0000_0000 };
        assert_eq!(wts_query(&s, WTS_CURRENT_SESSION, WTS_SESSION_ID), Ok(vec![1, 0, 0, 0]));
        assert_eq!(wts_query(&s, 1, WTS_CONNECT_STATE), Ok(vec![0, 0, 0, 0]));
        assert_eq!(wts_query(&s, 1, WTS_CLIENT_PROTOCOL_TYPE), Ok(vec![0, 0]));
        assert_eq!(wts_query(&s, 1, WTS_IS_REMOTE_SESSION), Ok(vec![0]));
        assert_eq!(wts_query(&s, 1, WTS_USER_NAME), Ok(vec![b'p', 0, b'o', 0, b'l', 0, b'y', 0, 0, 0]));
        assert_eq!(wts_query(&s, 1, WTS_INITIAL_PROGRAM), Ok(vec![0, 0]));
        let station = wts_query(&s, 1, WTS_WIN_STATION_NAME).unwrap();
        assert_eq!(station.len(), 16);
        assert_eq!(&station[..2], &[b'C', 0]);
        assert_eq!(wts_query(&s, 1, WTS_DOMAIN_NAME).unwrap().len(), 16);
        // Another session, a class without an answer.
        assert_eq!(wts_query(&s, 2, WTS_SESSION_ID), Err(1733));
        assert_eq!(wts_query(&s, 0x7FFF, WTS_CLIENT_PROTOCOL_TYPE), Ok(vec![0, 0]));
        assert_eq!(wts_query(&s, 1, 100), Err(ERROR_INVALID_PARAMETER));
        // WTSSessionInfoEx: 232 bytes, level 1, this session, active, unlocked, "Console".
        let ex = wts_query(&s, WTS_CURRENT_SESSION, WTS_SESSION_INFO_EX).unwrap();
        let d = |at: usize| u32::from_le_bytes(ex[at..at + 4].try_into().unwrap());
        assert_eq!((ex.len(), d(0), d(8), d(12), d(16)), (232, 1, 1, 0, 1));
        assert_eq!((ex[20], ex[22], ex[20 + 14]), (b'C', b'o', 0));
        assert_eq!((ex[86], ex[128]), (b'p', b'P'));
        assert_eq!(ex[207], 0x01);
    }

    #[test]
    fn session_notifications_need_a_window_and_a_known_flag() {
        assert_eq!(wts_register(0x1234, 0), Ok(()));
        assert_eq!(wts_register(0x1234, NOTIFY_FOR_ALL_SESSIONS), Ok(()));
        assert_eq!(wts_register(0, 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(wts_register(0x1234, 2), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn processor_information_is_one_record_per_processor() {
        let b = power_information(PROCESSOR_INFORMATION, 2, 48).unwrap();
        assert_eq!(b.len(), 48);
        let q = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        assert_eq!((q(0), q(4), q(8), q(12), q(16), q(20)), (0, 3000, 3000, 3000, 0, 0));
        assert_eq!(q(24), 1);
        assert_eq!(power_information(PROCESSOR_INFORMATION, 2, 47), Err(STATUS_BUFFER_TOO_SMALL));
        assert_eq!(power_information(PROCESSOR_INFORMATION, 2, 0), Err(STATUS_INVALID_PARAMETER));
        let caps = power_information(SYSTEM_POWER_CAPABILITIES, 2, 76).unwrap();
        assert_eq!((caps.len(), caps[0], caps[7]), (76, 1, 1));
        assert_eq!(power_information(PROCESSOR_INFORMATION, 0, 24).map(|b| b.len()), Ok(24));
    }

    #[test]
    fn the_machine_is_on_mains_power() {
        let b = power_information(SYSTEM_BATTERY_STATE, 4, 32).unwrap();
        assert_eq!(b.len(), 32);
        assert_eq!(b[0], 1);
        assert!(b[1..].iter().all(|&x| x == 0));
        assert_eq!(power_information(SYSTEM_BATTERY_STATE, 4, 8), Err(STATUS_BUFFER_TOO_SMALL));
        assert_eq!(power_information(12, 4, 64), Err(STATUS_NOT_SUPPORTED));
        assert_eq!(power_information(SYSTEM_EXECUTION_STATE, 4, 4), Ok(alloc::vec![0u8; 4]));
    }

    #[test]
    fn common_controls_want_the_structure_size() {
        assert!(init_common_controls(8));
        assert!(!init_common_controls(0));
        assert!(!init_common_controls(16));
    }
}
