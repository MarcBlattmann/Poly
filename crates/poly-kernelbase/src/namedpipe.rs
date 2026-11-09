//! Named pipes and overlapped I/O, as `kernelbase` presents them (Learn:
//! "CreateNamedPipeW", "ConnectNamedPipe", "PeekNamedPipe",
//! "WaitNamedPipeW", "GetOverlappedResult", "GetQueuedCompletionStatus",
//! "SetFileCompletionNotificationModes"; ADR 0169): the arguments of the
//! Win32 calls turned into the parameters of the NT services and back.
//! The flag values are `winbase.h`'s, the control codes `winioctl.h`'s and
//! the structures `ntifs.h`'s.

use crate::error::*;
use alloc::vec::Vec;

pub const PIPE_ACCESS_INBOUND: u32 = 1;
pub const PIPE_ACCESS_OUTBOUND: u32 = 2;
pub const PIPE_ACCESS_DUPLEX: u32 = 3;
pub const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;
pub const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
pub const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
pub const WRITE_DAC: u32 = 0x0004_0000;
pub const ACCESS_SYSTEM_SECURITY: u32 = 0x0100_0000;

pub const PIPE_WAIT: u32 = 0;
pub const PIPE_NOWAIT: u32 = 1;
pub const PIPE_READMODE_BYTE: u32 = 0;
pub const PIPE_READMODE_MESSAGE: u32 = 2;
pub const PIPE_TYPE_BYTE: u32 = 0;
pub const PIPE_TYPE_MESSAGE: u32 = 4;
pub const PIPE_ACCEPT_REMOTE_CLIENTS: u32 = 0;
pub const PIPE_REJECT_REMOTE_CLIENTS: u32 = 8;
pub const PIPE_UNLIMITED_INSTANCES: u32 = 255;
pub const PIPE_CLIENT_END: u32 = 0;
pub const PIPE_SERVER_END: u32 = 1;

pub const NMPWAIT_WAIT_FOREVER: u32 = 0xFFFF_FFFF;
pub const NMPWAIT_USE_DEFAULT_WAIT: u32 = 0;

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const SYNCHRONIZE: u32 = 0x0010_0000;
const FILE_SHARE_READ: u32 = 1;
const FILE_SHARE_WRITE: u32 = 2;
const FILE_CREATE: u32 = 2;
const FILE_OPEN_IF: u32 = 3;
const FILE_WRITE_THROUGH: u32 = 2;
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x20;

/// The prefix of a pipe name in Win32: `\\.\pipe\`.
const PREFIX: &str = "\\\\.\\pipe\\";

/// The parameters of `NtCreateNamedPipeFile` that `CreateNamedPipeW`'s
/// arguments make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServerParams {
    pub access: u32,
    pub share: u32,
    pub disposition: u32,
    pub options: u32,
    /// Bit 0 message type, bit 1 `FILE_PIPE_REJECT_REMOTE_CLIENTS`.
    pub pipe_type: u32,
    pub read_mode: u32,
    pub completion_mode: u32,
    pub max_instances: u32,
    pub inbound_quota: u32,
    pub outbound_quota: u32,
    /// The default time-out, relative (negative, 100 ns).
    pub timeout: i64,
}

/// `CreateNamedPipeW`'s argument checks and conversions. `nMaxInstances` is
/// 1 to 254, or `PIPE_UNLIMITED_INSTANCES`; the open mode needs a
/// direction; a message read mode needs a message pipe type; a default
/// time-out of 0 is 50 ms (Learn).
pub fn server_params(
    open_mode: u32,
    pipe_mode: u32,
    max_instances: u32,
    out_size: u32,
    in_size: u32,
    default_timeout_ms: u32,
) -> Result<ServerParams, u32> {
    let direction = open_mode & PIPE_ACCESS_DUPLEX;
    let known = PIPE_ACCESS_DUPLEX
        | FILE_FLAG_WRITE_THROUGH
        | FILE_FLAG_OVERLAPPED
        | FILE_FLAG_FIRST_PIPE_INSTANCE
        | WRITE_DAC
        | ACCESS_SYSTEM_SECURITY;
    if direction == 0 || open_mode & !known != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let known_modes =
        PIPE_NOWAIT | PIPE_READMODE_MESSAGE | PIPE_TYPE_MESSAGE | PIPE_REJECT_REMOTE_CLIENTS;
    if pipe_mode & !known_modes != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if pipe_mode & PIPE_READMODE_MESSAGE != 0 && pipe_mode & PIPE_TYPE_MESSAGE == 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if max_instances == 0 || max_instances > PIPE_UNLIMITED_INSTANCES {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let mut access = SYNCHRONIZE;
    let mut share = 0;
    if direction & PIPE_ACCESS_INBOUND != 0 {
        access |= GENERIC_READ;
        share |= FILE_SHARE_WRITE;
    }
    if direction & PIPE_ACCESS_OUTBOUND != 0 {
        access |= GENERIC_WRITE;
        share |= FILE_SHARE_READ;
    }
    if open_mode & WRITE_DAC != 0 {
        access |= WRITE_DAC;
    }
    if open_mode & ACCESS_SYSTEM_SECURITY != 0 {
        access |= ACCESS_SYSTEM_SECURITY;
    }
    let mut options = 0;
    if open_mode & FILE_FLAG_OVERLAPPED == 0 {
        options |= FILE_SYNCHRONOUS_IO_NONALERT;
    }
    if open_mode & FILE_FLAG_WRITE_THROUGH != 0 {
        options |= FILE_WRITE_THROUGH;
    }
    let ms = if default_timeout_ms == 0 { 50 } else { default_timeout_ms };
    Ok(ServerParams {
        access,
        share,
        disposition: if open_mode & FILE_FLAG_FIRST_PIPE_INSTANCE != 0 { FILE_CREATE } else { FILE_OPEN_IF },
        options,
        pipe_type: u32::from(pipe_mode & PIPE_TYPE_MESSAGE != 0)
            | if pipe_mode & PIPE_REJECT_REMOTE_CLIENTS != 0 { 2 } else { 0 },
        read_mode: u32::from(pipe_mode & PIPE_READMODE_MESSAGE != 0),
        completion_mode: u32::from(pipe_mode & PIPE_NOWAIT != 0),
        max_instances: if max_instances == PIPE_UNLIMITED_INSTANCES { u32::MAX } else { max_instances },
        inbound_quota: in_size,
        outbound_quota: out_size,
        timeout: -(i64::from(ms) * 10_000),
    })
}

/// A pipe name: `\\.\pipe\<name>` (the prefix in any case). The part after
/// the prefix is returned; a name with nothing after it is
/// `ERROR_INVALID_NAME`, one outside the pipe directory
/// `ERROR_FILE_NOT_FOUND` (Learn: "Named Pipe Names"), and anything that is
/// no device path `ERROR_BAD_PATHNAME`.
pub fn pipe_name(win32: &[u16]) -> Result<&[u16], u32> {
    let prefix: Vec<u16> = PREFIX.encode_utf16().collect();
    let dev: Vec<u16> = "\\\\.\\".encode_utf16().collect();
    let fold = |c: u16| if (u16::from(b'A')..=u16::from(b'Z')).contains(&c) { c + 32 } else { c };
    let starts = |p: &[u16]| win32.len() >= p.len() && win32.iter().zip(p).all(|(&a, &b)| fold(a) == fold(b));
    if starts(&prefix) {
        let rest = &win32[prefix.len()..];
        return if rest.is_empty() { Err(ERROR_INVALID_NAME) } else { Ok(rest) };
    }
    if starts(&dev) { Err(ERROR_FILE_NOT_FOUND) } else { Err(ERROR_BAD_PATHNAME) }
}

/// What `CreateNamedPipeW` leaves in the last error: `ERROR_ALREADY_EXISTS`
/// for a further instance (a success), else 0.
pub fn created_error(information: u64) -> u32 {
    if information == 1 { ERROR_ALREADY_EXISTS } else { ERROR_SUCCESS }
}

/// `FILE_PIPE_LOCAL_INFORMATION` (`ntifs.h`), 36 bytes of `ULONG`s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub pipe_type: u32,
    pub configuration: u32,
    pub maximum: u32,
    pub current: u32,
    pub inbound: u32,
    pub outbound: u32,
    pub available: u32,
    pub state: u32,
    pub end: u32,
}

impl Local {
    pub fn parse(b: &[u8]) -> Option<Local> {
        if b.len() < 36 {
            return None;
        }
        let d = |i: usize| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().expect("4 bytes"));
        Some(Local {
            pipe_type: d(0),
            configuration: d(1),
            maximum: d(2),
            current: d(3),
            inbound: d(4),
            outbound: d(5),
            available: d(6),
            state: d(7),
            end: d(8),
        })
    }

    /// `GetNamedPipeInfo`'s flags: the end, the type and the remote flag.
    pub fn flags(&self) -> u32 {
        (if self.end == 1 { PIPE_SERVER_END } else { PIPE_CLIENT_END })
            | if self.pipe_type & 1 != 0 { PIPE_TYPE_MESSAGE } else { 0 }
            | if self.pipe_type & 2 != 0 { PIPE_REJECT_REMOTE_CLIENTS } else { 0 }
    }

    /// The instance limit as `GetNamedPipeInfo` reports it: unlimited is 255.
    pub fn max_instances(&self) -> u32 {
        if self.maximum == u32::MAX { PIPE_UNLIMITED_INSTANCES } else { self.maximum }
    }
}

/// `GetNamedPipeHandleStateW`'s state from `FILE_PIPE_INFORMATION`
/// (read mode, completion mode).
pub fn handle_state(read_mode: u32, completion: u32) -> u32 {
    (if read_mode != 0 { PIPE_READMODE_MESSAGE } else { 0 }) | (if completion != 0 { PIPE_NOWAIT } else { 0 })
}

/// `SetNamedPipeHandleState`'s mode as `FILE_PIPE_INFORMATION`; bits that
/// are no mode bits are `ERROR_INVALID_PARAMETER`.
pub fn set_state(mode: u32) -> Result<[u8; 8], u32> {
    if mode & !(PIPE_NOWAIT | PIPE_READMODE_MESSAGE) != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let mut b = [0u8; 8];
    b[0..4].copy_from_slice(&u32::from(mode & PIPE_READMODE_MESSAGE != 0).to_le_bytes());
    b[4..8].copy_from_slice(&u32::from(mode & PIPE_NOWAIT != 0).to_le_bytes());
    Ok(b)
}

/// The input of `FSCTL_PIPE_WAIT` (`FILE_PIPE_WAIT_FOR_BUFFER`, `ntifs.h`):
/// the time-out, the name length, whether a time-out is given, the name.
/// `NMPWAIT_USE_DEFAULT_WAIT` leaves the pipe's default, `NMPWAIT_WAIT_FOREVER`
/// is the largest wait.
pub fn wait_input(name: &[u16], timeout_ms: u32) -> Vec<u8> {
    let (timeout, specified) = match timeout_ms {
        NMPWAIT_USE_DEFAULT_WAIT => (0i64, false),
        NMPWAIT_WAIT_FOREVER => (i64::MIN, true),
        ms => (-(i64::from(ms) * 10_000), true),
    };
    let mut b = timeout.to_le_bytes().to_vec();
    b.extend_from_slice(&((name.len() * 2) as u32).to_le_bytes());
    b.push(u8::from(specified));
    b.extend_from_slice(&[0, 0, 0]);
    for c in name {
        b.extend_from_slice(&c.to_le_bytes());
    }
    b
}

/// The header of a peek (`FILE_PIPE_PEEK_BUFFER`, `ntifs.h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peek {
    pub state: u32,
    pub available: u32,
    pub messages: u32,
    pub message_length: u32,
}

impl Peek {
    pub fn parse(b: &[u8]) -> Option<Peek> {
        if b.len() < 16 {
            return None;
        }
        let d = |i: usize| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().expect("4 bytes"));
        Some(Peek { state: d(0), available: d(1), messages: d(2), message_length: d(3) })
    }

    /// `lpBytesLeftThisMessage`: the first message's length less what was
    /// copied, as an unsigned 32-bit wrap (for a byte pipe the length is
    /// 0, so a copy shows `0 - copied`; Learn leaves it open, the spec sheet
    /// observed it).
    pub fn bytes_left(&self, copied: u32) -> u32 {
        self.message_length.wrapping_sub(copied)
    }
}

/// `SetFileCompletionNotificationModes` flags: `FILE_SKIP_COMPLETION_PORT_ON_SUCCESS`
/// (1) and `FILE_SKIP_SET_EVENT_ON_HANDLE` (2) only.
pub fn check_notification_flags(flags: u8) -> Result<(), u32> {
    if flags & !3 != 0 { Err(ERROR_INVALID_PARAMETER) } else { Ok(()) }
}

/// A time-out in milliseconds as `NtRemoveIoCompletion` and the waits take
/// it: `INFINITE` is none, 0 is a poll, else relative in 100 ns.
pub fn nt_timeout(ms: u32) -> Option<i64> {
    if ms == u32::MAX { None } else { Some(-(i64::from(ms) * 10_000)) }
}

/// What `GetOverlappedResult` makes of an `OVERLAPPED`'s status and byte
/// count once the request is no longer pending.
pub fn overlapped_outcome(status: u32, bytes: u64) -> Result<u32, (u32, u32)> {
    if (status as i32) >= 0 {
        Ok(bytes as u32)
    } else {
        Err((poly_rtl::status::nt_status_to_dos_error(poly_ntobj::NtStatus(status)), bytes as u32))
    }
}

/// `STATUS_PENDING`.
pub const STATUS_PENDING: u32 = 0x103;

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn create_named_pipe_arguments() {
        let p = server_params(PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED, PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE, 255, 1, 2, 0)
            .unwrap();
        assert_eq!(p.share, 3);
        assert_eq!(p.options, 0);
        assert_eq!((p.pipe_type, p.read_mode, p.completion_mode), (1, 1, 0));
        assert_eq!(p.max_instances, u32::MAX);
        assert_eq!((p.inbound_quota, p.outbound_quota), (2, 1));
        assert_eq!(p.timeout, -500_000);
        let p = server_params(PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS, 3, 0, 0, 7)
            .unwrap();
        assert_eq!((p.share, p.disposition, p.options), (2, 2, 0x20));
        assert_eq!((p.pipe_type, p.completion_mode, p.max_instances, p.timeout), (2, 1, 3, -70_000));
        assert_eq!(server_params(0, 0, 1, 0, 0, 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(server_params(1, 0, 0, 0, 0, 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(server_params(1, 0, 256, 0, 0, 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(server_params(1, PIPE_READMODE_MESSAGE, 1, 0, 0, 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(server_params(1, 0x10, 1, 0, 0, 0), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn names() {
        assert_eq!(pipe_name(&w("\\\\.\\PIPE\\abc")).unwrap(), &w("abc")[..]);
        assert_eq!(pipe_name(&w("\\\\.\\pipe\\")), Err(ERROR_INVALID_NAME));
        assert_eq!(pipe_name(&w("\\\\.\\other\\x")), Err(ERROR_FILE_NOT_FOUND));
        assert_eq!(pipe_name(&w("plain")), Err(ERROR_BAD_PATHNAME));
    }

    #[test]
    fn info_and_state() {
        let mut b = Vec::new();
        for v in [1u32 | 2, 2, u32::MAX, 3, 4096, 8192, 0, 3, 1] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let l = Local::parse(&b).unwrap();
        assert_eq!(l.flags(), PIPE_SERVER_END | PIPE_TYPE_MESSAGE | PIPE_REJECT_REMOTE_CLIENTS);
        assert_eq!(l.max_instances(), 255);
        assert_eq!(handle_state(1, 1), 3);
        assert_eq!(set_state(2).unwrap(), [1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(set_state(1).unwrap(), [0, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(set_state(4), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn wait_and_peek() {
        let b = wait_input(&w("p"), 100);
        assert_eq!(i64::from_le_bytes(b[0..8].try_into().unwrap()), -1_000_000);
        assert_eq!((b[8], b[12], b.len()), (2, 1, 18));
        assert_eq!(wait_input(&w("p"), NMPWAIT_USE_DEFAULT_WAIT)[12], 0);
        assert_eq!(i64::from_le_bytes(wait_input(&w("p"), NMPWAIT_WAIT_FOREVER)[0..8].try_into().unwrap()), i64::MIN);
        let p = Peek { state: 3, available: 11, messages: 0, message_length: 0 };
        assert_eq!(p.bytes_left(5), 4_294_967_291);
        assert_eq!(p.bytes_left(0), 0);
        assert_eq!(Peek { message_length: 3, ..p }.bytes_left(2), 1);
    }

    #[test]
    fn overlapped_results() {
        assert_eq!(overlapped_outcome(0, 7), Ok(7));
        assert_eq!(overlapped_outcome(0xC000_014B, 0), Err((ERROR_BROKEN_PIPE, 0)));
        assert_eq!(overlapped_outcome(0x8000_0005, 4), Err((ERROR_MORE_DATA, 4)));
        assert_eq!(nt_timeout(0), Some(0));
        assert_eq!(nt_timeout(u32::MAX), None);
        assert_eq!(check_notification_flags(3), Ok(()));
        assert_eq!(check_notification_flags(4), Err(ERROR_INVALID_PARAMETER));
    }
}
