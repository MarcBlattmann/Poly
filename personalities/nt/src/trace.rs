//! The NT call trace (`poly.nt.trace`, ADR 0155): one line per system
//! call at the boundary between `ntdll` and the kernel, so what a program
//! asked for and what it was told can be read from the serial log (Chrome's
//! bring-up, ADR 0155; the comparer is `cargo xtask win chrome-trace`).
//!
//! ```text
//! nt-trace: p2.t7 NtOpenKey(0x1f0, 0x20019, 0x8f1e0) "\Registry\Machine\Software\Policies" -> 0xc0000034
//! ```
//!
//! The kernel decides whether tracing is on and prints the line; this
//! module reads the arguments (and, for the services that take a name, the
//! name from the `OBJECT_ATTRIBUTES` or `UNICODE_STRING`) and formats the
//! line. Services that are called constantly and almost never interesting
//! ([`is_noisy`]) are left out unless they fail, or the trace is `all`.

use crate::abi;
use crate::kernel::NtKernel;
use crate::services::Service;
use alloc::format;
use alloc::string::String;
use poly_abi::SyscallRegs;

/// The kernel argument that turns the trace on; `poly.nt.trace=all` adds
/// the noisy services.
pub const FLAG: &str = "poly.nt.trace";

/// `poly.nt.only=<text>`: the trace and the trail are kept only for processes whose command line
/// contains `text` (Chrome's `--type=renderer`, ADR 0155 step 11).
pub const ONLY_FLAG: &str = "poly.nt.only";

/// `poly.nt.trail[=N]`: every process keeps its last N system calls (all of them, the noisy ones
/// too) and they are written when an exception that is a breakpoint or an error is handed to it
/// (`nt-trail:` lines, oldest first): the calls before a failed check, without a serial line per
/// call. N is [`TRAIL_DEFAULT`] without a value.
pub const TRAIL_FLAG: &str = "poly.nt.trail";
pub const TRAIL_DEFAULT: usize = 64;
/// The most calls a trail keeps.
pub const TRAIL_MAX: usize = 4096;

/// The trail length `poly.nt.trail` asks for: `None` without the flag.
pub fn trail_len(flag_present: bool, value: Option<&str>) -> Option<usize> {
    match value {
        Some(v) => v.parse::<usize>().ok().filter(|&n| n > 0).map(|n| n.min(TRAIL_MAX)),
        None => flag_present.then_some(TRAIL_DEFAULT),
    }
}

/// Whether a process with `command_line` is traced when `poly.nt.only` is `only` (empty: every
/// process; `a|b`: a command line with either text). A process whose command line is not known
/// yet is not.
pub fn selects(command_line: Option<&str>, only: &str) -> bool {
    only.is_empty() || command_line.is_some_and(|c| only.split('|').any(|o| !o.is_empty() && c.contains(o)))
}

/// Whether an exception with `code` makes the trail be written: a breakpoint
/// (`STATUS_BREAKPOINT`, a failed check's `int3`) or an error status (`0xC...`).
pub fn writes_trail(code: u32) -> bool {
    code == 0x8000_0003 || code >= 0xC000_0000
}

/// The last calls of one process (see [`TRAIL_FLAG`]).
#[derive(Clone, Debug, Default)]
pub struct Trail {
    lines: alloc::collections::VecDeque<String>,
}

impl Trail {
    /// Keep `line`, forgetting the oldest beyond `cap`.
    pub fn push(&mut self, line: String, cap: usize) {
        while self.lines.len() >= cap.max(1) {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    /// The kept calls, oldest first; the trail starts over.
    pub fn take(&mut self) -> alloc::vec::Vec<String> {
        self.lines.drain(..).collect()
    }

    /// The kept calls, oldest first; they stay (a periodic report, `poly.nt.waits`).
    pub fn lines(&self) -> impl Iterator<Item = &String> {
        self.lines.iter()
    }
}

/// Where a service takes a name, and in what form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameKind {
    /// A pointer to an `OBJECT_ATTRIBUTES` (`ntdef.h`).
    ObjectAttributes,
    /// A pointer to a `UNICODE_STRING`.
    UnicodeString,
}

/// The argument of `service` that names something, if one does (`NtCreateFile`'s
/// third argument is `ObjectAttributes`, `NtQueryValueKey`'s second the value
/// name: the WDK prototypes in the docs on each [`Service`]).
pub fn name_site(service: Service) -> Option<(usize, NameKind)> {
    use NameKind::*;
    use Service::*;
    Some(match service {
        CreateFile
        | OpenFile
        | CreateKey
        | OpenKey
        | OpenKeyEx
        | CreateSection
        | OpenSection
        | CreateEvent
        | OpenEvent
        | CreateMutant
        | OpenMutant
        | CreateSemaphore
        | OpenSemaphore
        | CreateTimer
        | OpenTimer
        | CreateDirectoryObject
        | OpenDirectoryObject
        | CreateSymbolicLinkObject
        | OpenSymbolicLinkObject
        | OpenProcess
        | OpenThread => (2, ObjectAttributes),
        QueryAttributesFile | QueryFullAttributesFile => (0, ObjectAttributes),
        QueryValueKey | SetValueKey | DeleteValueKey => (1, UnicodeString),
        _ => return None,
    })
}

/// Services too frequent to trace without being asked to (waits, events,
/// the clock, virtual memory, handles): shown when they fail, or with `all`.
pub fn is_noisy(service: Service) -> bool {
    use Service::*;
    matches!(
        service,
        Close
            | SetEvent
            | ResetEvent
            | WaitForSingleObject
            | WaitForMultipleObjects
            | SignalAndWaitForSingleObject
            | DelayExecution
            | YieldExecution
            | QueryPerformanceCounter
            | QuerySystemTime
            | AllocateVirtualMemory
            | FreeVirtualMemory
            | ProtectVirtualMemory
            | QueryVirtualMemory
            | ReleaseSemaphore
            | ReleaseMutant
            | QueueApcThread
            | TestAlert
            | PolyDebugPrint
            | PolyTrace
            | RaiseException
            | Continue
    )
}

/// A call as it was made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    /// `NtCreateFile`, or `win32k#12` / `nt#0x3fff` for a service we cannot name.
    pub name: String,
    pub service: Option<Service>,
    /// The first six arguments (a stack argument that cannot be read is 0).
    pub args: [u64; 6],
    /// The name the call carries, when it has one and it could be read.
    pub object: Option<String>,
}

/// Read the call in `regs` (before the service runs).
pub fn begin<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Call {
    let number = regs.rax as u32;
    let service = Service::from_number(number);
    let name = match service {
        Some(s) => s.name().into(),
        None => {
            let (table, index) = crate::decode(number);
            format!("{}#{index:#x}", table.name())
        }
    };
    let mut args = [0u64; 6];
    for (i, slot) in args.iter_mut().enumerate() {
        *slot = abi::arg(k, regs, i).unwrap_or(0);
    }
    let object = service.and_then(name_site).and_then(|(i, kind)| match kind {
        NameKind::ObjectAttributes => {
            abi::read_attributes(k, args[i]).ok().and_then(|a| a.name).map(|n| String::from_utf16_lossy(&n))
        }
        NameKind::UnicodeString => {
            if args[i] == 0 {
                None
            } else {
                abi::read_unicode(k, args[i]).ok().map(|n| String::from_utf16_lossy(&n))
            }
        }
    });
    // `PolyUserCall` (`win32k#0x0`): the request it carries, so a trail
    // shows which user32 call it was.
    let object = object.or_else(|| if service.is_some() { None } else { user_call_request(k, &name, &args) });
    Call { name, service, args, object }
}

/// The request a `PolyUserCall` (`win32k#0x0`) carries, as text (at most 200 characters), if the
/// call is one and its request is a readable `poly_win32k::proto::Request`.
fn user_call_request<K: NtKernel>(k: &mut K, name: &str, args: &[u64; 6]) -> Option<String> {
    if name != "win32k#0x0" || args[1] == 0 || args[1] > 4096 {
        return None;
    }
    let mut bytes = alloc::vec![0u8; args[1] as usize];
    abi::read(k, args[0], &mut bytes).ok()?;
    let req: poly_win32k::proto::Request = poly_win32k::wire::decode(&bytes)?;
    let mut text = format!("{req:?}");
    if text.len() > 200 {
        let mut end = 200;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("...");
    }
    Some(text)
}

/// The fields of a 32-bit `UNICODE_STRING` header (`Length`, `MaximumLength`, a 4-byte buffer
/// pointer; 8 bytes): the length in bytes and the buffer's address.
pub fn unicode_string32(header: &[u8; 8]) -> (u16, u32) {
    (u16::from_le_bytes([header[0], header[1]]), u32::from_le_bytes([header[4], header[5], header[6], header[7]]))
}

/// The `ObjectName` pointer of a 32-bit `OBJECT_ATTRIBUTES` (24 bytes: `Length`,
/// `RootDirectory`, `ObjectName`, `Attributes`, `SecurityDescriptor`, `SecurityQualityOfService`).
pub fn object_name32(attributes: &[u8; 24]) -> u32 {
    u32::from_le_bytes([attributes[8], attributes[9], attributes[10], attributes[11]])
}

/// Read the 32-bit string whose `UNICODE_STRING32` is at `at`.
fn read_string32<K: NtKernel>(k: &mut K, at: u64) -> Option<String> {
    let mut header = [0u8; 8];
    abi::read(k, at, &mut header).ok()?;
    let (length, buffer) = unicode_string32(&header);
    if length == 0 || length % 2 != 0 || length > 2048 {
        return None;
    }
    let mut bytes = alloc::vec![0u8; length as usize];
    abi::read(k, u64::from(buffer), &mut bytes).ok()?;
    let units: alloc::vec::Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(String::from_utf16_lossy(&units))
}

/// Read a call made by a 32-bit thread (ADR 0186): the arguments are 4-byte words at `EDX`
/// (`int 0xF4`, EAX = the number), and names are in the 32-bit layouts.
pub fn begin32<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Call {
    let number = regs.rax as u32;
    let service = Service::from_number(number);
    let name = match service {
        Some(s) => s.name().into(),
        None => {
            let (table, index) = crate::decode(number);
            format!("{}#{index:#x}", table.name())
        }
    };
    let mut raw = [0u8; 24];
    let _ = abi::read(k, regs.rdx, &mut raw);
    let mut args = [0u64; 6];
    for (i, slot) in args.iter_mut().enumerate() {
        *slot = u64::from(u32::from_le_bytes([raw[4 * i], raw[4 * i + 1], raw[4 * i + 2], raw[4 * i + 3]]));
    }
    let object = service.and_then(name_site).and_then(|(i, kind)| {
        if args[i] == 0 {
            return None;
        }
        match kind {
            NameKind::ObjectAttributes => {
                let mut attributes = [0u8; 24];
                abi::read(k, args[i], &mut attributes).ok()?;
                let at = object_name32(&attributes);
                if at == 0 { None } else { read_string32(k, u64::from(at)) }
            }
            NameKind::UnicodeString => read_string32(k, args[i]),
        }
    });
    let object = object.or_else(|| if service.is_some() { None } else { user_call_request(k, &name, &args) });
    Call { name, service, args, object }
}

/// How many of a service's arguments are worth printing: the documented
/// count would be exact, four covers the handle, the class and the buffer
/// of most.
const SHOWN: usize = 4;

/// The trace line for a call that returned `status` in process `pid`,
/// thread `tid`.
pub fn render(pid: u32, tid: u32, call: &Call, status: u32) -> String {
    let mut out = format!("nt-trace: p{pid}.t{tid} {}(", call.name);
    for (i, a) in call.args.iter().take(SHOWN).enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&format!("{a:#x}"));
    }
    out.push(')');
    if let Some(name) = &call.object {
        out.push_str(" \"");
        for c in name.chars() {
            match c {
                '\n' => out.push_str("\\n"),
                '"' => out.push_str("\\\""),
                c if c.is_control() => out.push('?'),
                c => out.push(c),
            }
        }
        out.push('"');
    }
    out.push_str(&format!(" -> {status:#x}"));
    out
}

/// Whether a call is printed: not a noisy service unless it failed (a
/// negative status is `>= 0xC0000000`; warnings and informational statuses
/// of the `0x4...`/`0x8...` kinds are shown too) or `all` was asked for.
pub fn wanted(call: &Call, status: u32, all: bool) -> bool {
    all || status >= 0x8000_0000 || !call.service.is_some_and(is_noisy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(service: Option<Service>, name: &str, object: Option<&str>) -> Call {
        Call { name: name.into(), service, args: [0x1f0, 0x20019, 0x8f1e0, 0, 7, 8], object: object.map(Into::into) }
    }

    #[test]
    fn a_32_bit_name_is_read_from_the_4_byte_layouts() {
        assert_eq!(unicode_string32(&[8, 0, 10, 0, 0x78, 0x56, 0x34, 0x12]), (8, 0x1234_5678));
        let mut oa = [0u8; 24];
        oa[0] = 24;
        oa[8..12].copy_from_slice(&0x0012_F000u32.to_le_bytes());
        assert_eq!(object_name32(&oa), 0x0012_F000);
    }

    #[test]
    fn a_line_has_the_process_thread_arguments_name_and_status() {
        let c = call(Some(Service::OpenKey), "NtOpenKey", Some("\\Registry\\Machine\\Software"));
        assert_eq!(
            render(2, 7, &c, 0xC000_0034),
            "nt-trace: p2.t7 NtOpenKey(0x1f0, 0x20019, 0x8f1e0, 0x0) \"\\Registry\\Machine\\Software\" -> 0xc0000034"
        );
        let c = call(None, "win32k#0x3", None);
        assert_eq!(render(1, 1, &c, 0), "nt-trace: p1.t1 win32k#0x3(0x1f0, 0x20019, 0x8f1e0, 0x0) -> 0x0");
    }

    #[test]
    fn names_are_escaped_onto_one_line() {
        let c = call(Some(Service::CreateFile), "NtCreateFile", Some("a\"b\nc\u{1}"));
        assert!(render(1, 1, &c, 0).contains("\"a\\\"b\\nc?\""));
    }

    #[test]
    fn noisy_services_show_only_when_they_fail_or_all_is_asked() {
        let wait = call(Some(Service::WaitForSingleObject), "NtWaitForSingleObject", None);
        assert!(!wanted(&wait, 0, false));
        assert!(!wanted(&wait, 0x102, false), "a timeout (0x102) is not a failure");
        assert!(wanted(&wait, 0xC000_0008, false));
        assert!(wanted(&wait, 0, true));
        let open = call(Some(Service::OpenKey), "NtOpenKey", None);
        assert!(wanted(&open, 0, false));
        assert!(wanted(&call(None, "nt#0x3fff", None), 0xC000_0002, false));
    }

    #[test]
    fn the_trail_keeps_the_last_calls_and_starts_over() {
        let mut t = Trail::default();
        for i in 0..5 {
            t.push(format!("call {i}"), 3);
        }
        assert_eq!(t.lines().cloned().collect::<alloc::vec::Vec<_>>(), ["call 2", "call 3", "call 4"]);
        assert_eq!(t.take(), ["call 2", "call 3", "call 4"]);
        assert!(t.take().is_empty());
        assert_eq!(trail_len(true, None), Some(TRAIL_DEFAULT));
        assert_eq!(trail_len(false, Some("200")), Some(200));
        assert_eq!(trail_len(false, Some("999999")), Some(TRAIL_MAX));
        assert_eq!(trail_len(false, Some("0")), None);
        assert_eq!(trail_len(false, None), None);
        assert!(writes_trail(0x8000_0003) && writes_trail(0xC000_0005));
        assert!(!writes_trail(0x406D_1388) && !writes_trail(0xE06D_7363 & 0x7FFF_FFFF));
    }

    #[test]
    fn only_names_processes_by_their_command_line() {
        let renderer = Some("\"C:\\Program Files\\Chrome\\chrome.exe\" --type=renderer --lang=en-US");
        assert!(selects(renderer, "--type=renderer"));
        assert!(!selects(Some("chrome.exe --type=gpu-process"), "--type=renderer"));
        assert!(!selects(None, "--type=renderer"));
        assert!(selects(None, ""));
        assert!(selects(Some("chrome.exe --dump-dom x"), "--type=renderer|--dump-dom"));
        assert!(!selects(Some("chrome.exe --type=gpu-process"), "--type=renderer|--dump-dom"));
    }

    #[test]
    fn the_services_that_take_names() {
        assert_eq!(name_site(Service::CreateFile), Some((2, NameKind::ObjectAttributes)));
        assert_eq!(name_site(Service::QueryFullAttributesFile), Some((0, NameKind::ObjectAttributes)));
        assert_eq!(name_site(Service::QueryValueKey), Some((1, NameKind::UnicodeString)));
        assert_eq!(name_site(Service::Close), None);
    }
}
