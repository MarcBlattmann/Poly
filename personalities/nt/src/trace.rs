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
    fn the_services_that_take_names() {
        assert_eq!(name_site(Service::CreateFile), Some((2, NameKind::ObjectAttributes)));
        assert_eq!(name_site(Service::QueryFullAttributesFile), Some((0, NameKind::ObjectAttributes)));
        assert_eq!(name_site(Service::QueryValueKey), Some((1, NameKind::UnicodeString)));
        assert_eq!(name_site(Service::Close), None);
    }
}
