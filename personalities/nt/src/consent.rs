//! The consent broker of User Account Control (ADR 0222).
//!
//! A program that is not elevated asks for administrator rights through `ShellExecute(Ex)`
//! (`runas`, or an executable whose manifest requires them). Our `shell32` calls
//! `PolyRequestElevation(Path, PathLength, Token*)`; the kernel decides who answers:
//!
//! - the test hook `poly.uac.answer=yes|no` ([`Broker::hook`]), never set by default: test runs
//!   have nobody at the screen;
//! - else the desktop: the compositor shows its own consent prompt (program, publisher, path,
//!   Yes and No; the rest of the desktop dimmed, input only from the real devices) for a
//!   [`ToCompositor::Consent`](poly_win32k::link::ToCompositor::Consent) record and answers with
//!   `FromCompositor::Consent`;
//! - with no desktop connected, nobody can answer: No.
//!
//! The calling thread waits for the answer (Windows' prompt blocks its caller too). Yes gives
//! the caller a handle to a primary token: the full token of its split token, marked as
//! consented ([`Token::elevated_for_consent`]), which `CreateProcessAsUserW` may use once to
//! start one process of a higher integrity than the caller's
//! ([`crate::security`]'s `check_given_token`). No is `STATUS_CANCELLED`, which `shell32` turns
//! into `ERROR_CANCELLED` and `hInstApp` 5. The prompt's texts come from the kernel (the path
//! the program named, its file name), never from the app's windows, so an app cannot dress the
//! prompt up; the app gets no handle to the prompt and cannot send it input.
//!
//! [`Token::elevated_for_consent`]: poly_security::token::Token::elevated_for_consent

use crate::abi;
use crate::kernel::{NtKernel, NtTimeout};
use crate::objects::return_handle;
use crate::system::NtSystem;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use poly_abi::SyscallRegs;
use poly_ntobj::status::*;
use poly_ntobj::{EventType, NewObject, NtStatus, ObjectId, ObjectType, ProcessId, WaitOutcome, WaitType};
use poly_security::token;

type Result<T> = core::result::Result<T, NtStatus>;

/// `STATUS_CANCELLED` (`ntstatus.h`): the user said no.
pub const STATUS_CANCELLED: NtStatus = NtStatus(0xC000_0120);

/// The kernel argument of the test hook: `yes` or `no` answers every prompt at once.
pub const ANSWER_ARG: &str = "poly.uac.answer";

/// The longest path a request may name, in UTF-16 units.
const MAX_PATH_UNITS: usize = 32_767;

/// One open request.
#[derive(Debug)]
struct Pending {
    /// The notification event the asking thread waits on (the broker holds a reference).
    event: ObjectId,
    pid: ProcessId,
    answer: Option<bool>,
}

/// The requests waiting for an answer, and how they are answered.
#[derive(Debug, Default)]
pub struct Broker {
    /// `poly.uac.answer`: every request answered at once (test runs).
    pub hook: Option<bool>,
    pending: BTreeMap<u32, Pending>,
    next: u32,
    /// Every request and its answer, for the kernel log: `(request, program, answer)`.
    pub log: Vec<(u32, String, Option<bool>)>,
}

impl Broker {
    /// The value of `poly.uac.answer`.
    pub fn parse_hook(value: &str) -> Option<bool> {
        match value {
            "yes" => Some(true),
            "no" => Some(false),
            _ => None,
        }
    }

    fn answer(&mut self, objects: &mut poly_ntobj::ObjectManager, request: u32, allow: bool) {
        if let Some(p) = self.pending.get_mut(&request)
            && p.answer.is_none()
        {
            p.answer = Some(allow);
            let _ = objects.set_event(p.event);
            if let Some(entry) = self.log.iter_mut().rev().find(|e| e.0 == request) {
                entry.2 = Some(allow);
            }
        }
    }

    /// The desktop answered `request`.
    pub fn desktop_answered(&mut self, objects: &mut poly_ntobj::ObjectManager, request: u32, allow: bool) {
        self.answer(objects, request, allow);
    }

    /// The desktop went away: nobody can answer what it was asked (No).
    pub fn desktop_gone(&mut self, objects: &mut poly_ntobj::ObjectManager) {
        let open: Vec<u32> = self.pending.iter().filter(|(_, p)| p.answer.is_none()).map(|(&r, _)| r).collect();
        for r in open {
            self.answer(objects, r, false);
        }
    }

    /// Process `pid` ended: its requests go (a prompt still shown is answered No for nobody).
    pub fn process_exited(&mut self, objects: &mut poly_ntobj::ObjectManager, pid: ProcessId) {
        let gone: Vec<u32> = self.pending.iter().filter(|(_, p)| p.pid == pid).map(|(&r, _)| r).collect();
        for r in gone {
            if let Some(p) = self.pending.remove(&r) {
                let _ = objects.dereference(p.event);
            }
        }
    }
}

/// The file name of a DOS path (the part after the last separator).
fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Open request `request`: decide it at once or ask the desktop. Returns the event to wait on.
fn ask(
    sys: &mut NtSystem,
    pid: ProcessId,
    path: &str,
    file: Option<String>,
    thread: poly_ntobj::ThreadRef,
) -> Result<(u32, ObjectId)> {
    let event = sys.objects.new_object(NewObject::Event { kind: EventType::Notification, signaled: false }, thread)?;
    // The broker holds one reference, the waiting call the other.
    sys.objects.reference(event)?;
    sys.consent.next = sys.consent.next.wrapping_add(1).max(1);
    let request = sys.consent.next;
    sys.consent.pending.insert(request, Pending { event, pid, answer: None });
    let program = String::from(file_name(path));
    sys.consent.log.push((request, program.clone(), None));
    if let Some(a) = sys.consent.hook {
        sys.consent.answer(&mut sys.objects, request, a);
    } else if sys.win32k.desktop.is_connected() {
        sys.win32k.desktop.ask_consent(request, pid.0, &program, path, file.as_deref().unwrap_or(""));
    } else {
        sys.consent.answer(&mut sys.objects, request, false);
    }
    Ok((request, event))
}

/// `PolyRequestElevation(Path, PathLength, Token*)` (Poly's own, ADR 0222): ask the user whether
/// the program at `Path` (`PathLength` UTF-16 units, a DOS path) may run with administrator
/// rights, and wait for the answer. Yes: `STATUS_SUCCESS` and a handle (all access) to a primary
/// full token marked as consented in `*Token`. No: `STATUS_CANCELLED`. A caller that is elevated
/// already gets its own full token without a prompt; a token that has no full token to give
/// (not an administrator's) is `STATUS_ACCESS_DENIED`.
pub fn request_elevation<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [path, length, out] = abi::args::<K, 3>(k, regs)?;
    let units = length as usize;
    if units == 0 || units > MAX_PATH_UNITS || out == 0 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let mut raw = alloc::vec![0u8; units * 2];
    abi::read(k, path, &mut raw)?;
    let wide: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let text = String::from_utf16_lossy(&wide);
    let caller = k.caller();
    // The caller's own token decides whether there is anything to ask.
    let (source, elevated) = k.with_state(|sys, _, _| {
        let id = crate::security::process_token(sys, caller.process, caller.thread)?;
        let t = sys.objects.security.token(id).ok_or(STATUS_INVALID_HANDLE)?;
        if t.elevation_type != token::TOKEN_ELEVATION_TYPE_LIMITED && !t.elevated {
            return Err(STATUS_ACCESS_DENIED);
        }
        Ok((id, t.elevated))
    })?;
    if !elevated {
        let (request, event, wait) = k.with_state(|sys, _, _| {
            let file = sys.io.vfs_path_of_dos(&wide).map(|b| String::from_utf8_lossy(&b).into_owned());
            let (request, event) = ask(sys, caller.process, &text, file, caller.thread)?;
            let wait =
                sys.objects.wait_objects(caller.thread, &[event], WaitType::Any, false, poly_ntobj::Timeout::Infinite);
            Ok::<_, NtStatus>((request, event, wait))
        })?;
        let _ = match wait {
            Ok(WaitOutcome::Done(s)) => s,
            Ok(WaitOutcome::Blocked(id)) => k.wait(id, NtTimeout::Infinite, false),
            Err(e) => e,
        };
        let allow = k.with_state(|sys, _, b| {
            let allow = sys.consent.pending.remove(&request).and_then(|p| {
                let _ = sys.objects.dereference(p.event);
                p.answer
            });
            let _ = sys.objects.dereference(event);
            sys.reap(b);
            allow.unwrap_or(false)
        });
        k.debug_print(
            alloc::format!("uac: request {request} for {text}: {}", if allow { "yes" } else { "no" }).as_bytes(),
        );
        if !allow {
            return Ok(STATUS_CANCELLED);
        }
    }
    let handle = k.with_state(|sys, _, b| {
        let id = sys.objects.security.new_luid();
        let full =
            sys.objects.security.token(source).and_then(|t| t.elevated_for_consent(id)).ok_or(STATUS_ACCESS_DENIED)?;
        let (_, obj) = sys.objects.new_token(full, caller.thread)?;
        sys.objects.assign_security(obj, ObjectType::Token, Some(source), None);
        let kernel = poly_ntobj::Caller::kernel(caller.process, caller.thread);
        let h = sys.objects.open_by_pointer(&kernel, obj, token::TOKEN_ALL_ACCESS, 0);
        let _ = sys.objects.dereference(obj);
        sys.reap(b);
        h
    })?;
    return_handle(k, out, handle)?;
    Ok(STATUS_SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::Service;
    use crate::tests::Fake;
    use poly_security::token::Logon;

    /// A process of `logon` whose prompts are answered `hook` asks for `C:\setup.exe`.
    fn ask_as(logon: Logon, hook: Option<bool>) -> (Fake, NtStatus, u64) {
        let mut f = Fake::new();
        f.sys.logon = logon;
        f.sys.consent.hook = hook;
        let path: Vec<u8> = r"C:\setup.exe".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let p = f.put(&path);
        let out = f.alloc(8);
        let st = f.call(Service::PolyRequestElevation, &[p, (path.len() / 2) as u64, out]);
        let h = f.u64_at(out);
        (f, st, h)
    }

    /// The integrity level and elevation of the token behind handle `h`, and its consent mark.
    fn facts(f: &mut Fake, h: u64) -> (u32, bool, bool) {
        let caller = f.caller();
        let obj = f.sys.objects.object_by_handle(&caller, poly_ntobj::Handle(h), 0, Some(ObjectType::Token)).unwrap();
        let id = f.sys.objects.cookie(obj).unwrap();
        let t = f.sys.objects.security.token(id).unwrap();
        (t.integrity_rid(), t.elevated, t.consented)
    }

    #[test]
    fn a_yes_gives_a_consented_full_token() {
        let (mut f, st, h) = ask_as(Logon::Standard, Some(true));
        assert_eq!(st, STATUS_SUCCESS);
        assert_eq!(facts(&mut f, h), (0x3000, true, true));
        assert_eq!(f.sys.consent.log.last().map(|e| (e.1.as_str(), e.2)), Some(("setup.exe", Some(true))));
        assert!(f.sys.consent.pending.is_empty());
    }

    #[test]
    fn a_no_or_nobody_to_ask_is_cancelled() {
        assert_eq!(ask_as(Logon::Standard, Some(false)).1, STATUS_CANCELLED);
        // No hook and no desktop connected: nobody can answer.
        let (f, st, _) = ask_as(Logon::Standard, None);
        assert_eq!(st, STATUS_CANCELLED);
        assert_eq!(f.sys.consent.log.last().map(|e| e.2), Some(Some(false)));
        assert!(f.sys.consent.pending.is_empty());
    }

    #[test]
    fn an_elevated_caller_is_not_asked() {
        for logon in [Logon::Elevated, Logon::Admin] {
            let (mut f, st, h) = ask_as(logon, Some(false));
            assert_eq!(st, STATUS_SUCCESS, "{logon:?}");
            assert_eq!(facts(&mut f, h).0, 0x3000);
            assert!(f.sys.consent.log.is_empty());
        }
    }

    #[test]
    fn the_desktop_answers_and_a_closed_desktop_says_no() {
        let mut b = Broker::default();
        let mut f = Fake::new();
        let t = f.caller().thread;
        let ev =
            f.sys.objects.new_object(NewObject::Event { kind: EventType::Notification, signaled: false }, t).unwrap();
        b.pending.insert(1, Pending { event: ev, pid: ProcessId(1), answer: None });
        b.pending.insert(2, Pending { event: ev, pid: ProcessId(1), answer: None });
        b.desktop_answered(&mut f.sys.objects, 1, true);
        b.desktop_gone(&mut f.sys.objects);
        assert_eq!(b.pending[&1].answer, Some(true));
        assert_eq!(b.pending[&2].answer, Some(false));
        assert_eq!(Broker::parse_hook("yes"), Some(true));
        assert_eq!(Broker::parse_hook("no"), Some(false));
        assert_eq!(Broker::parse_hook("maybe"), None);
    }
}
