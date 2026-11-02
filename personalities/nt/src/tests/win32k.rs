//! Table 1 (win32k, ADR 0128): `PolyUserCall` carries window-manager
//! requests through user memory, posting wakes the target thread's
//! registered event, the win32k lockdown refuses everything, unknown
//! numbers stay unimplemented.

use super::*;
use crate::STATUS_NOT_IMPLEMENTED;
use crate::win32k::{LOCKDOWN_STATUS, service_name};
use poly_abi::Outcome;
use poly_ntobj::status::STATUS_BUFFER_TOO_SMALL;
use poly_win32k::proto::{ClassSpec, Reply, Request};
use poly_win32k::services::Win32kService;
use poly_win32k::wire;

/// Make the system call `number` as a `win32u` stub would.
fn raw(f: &mut Fake, number: u32, args: &[u64]) -> (Outcome, u32) {
    let mut regs = SyscallRegs { rax: u64::from(number), rsp: STACK, ..Default::default() };
    let slots = [&mut regs.r10, &mut regs.rdx, &mut regs.r8, &mut regs.r9];
    for (slot, &a) in slots.into_iter().zip(args) {
        *slot = a;
    }
    for (i, &a) in args.iter().enumerate().skip(4) {
        f.mem.write_bytes(STACK + 0x28 + 8 * (i as u64 - 4), &a.to_le_bytes()).unwrap();
    }
    let outcome = crate::dispatch_with(f, &mut regs);
    (outcome, regs.rax as u32)
}

/// One `PolyUserCall` round trip.
fn user_call(f: &mut Fake, req: &Request, cap: usize) -> (NtStatus, Option<Reply>, u64) {
    let bytes = wire::encode(req);
    let req_ptr = f.put(&bytes);
    let out = f.alloc(cap.max(1));
    let len = f.put_u64(0);
    let (outcome, status) = raw(f, Win32kService::Call.number(), &[req_ptr, bytes.len() as u64, out, cap as u64, len]);
    assert_eq!(outcome, Outcome::Handled);
    let n = f.u64_at(len);
    let reply = (status == 0).then(|| wire::decode(&f.bytes_at(out, n as usize)).expect("a reply"));
    (NtStatus(status), reply, n)
}

#[test]
fn requests_round_trip_through_user_memory() {
    let mut f = Fake::new();
    let (s, r, _) = user_call(&mut f, &Request::Desktop {}, 256);
    assert_eq!(s, STATUS_SUCCESS);
    assert!(matches!(r, Some(Reply::Desktop { width: 1024, height: 768, .. })));
    let spec = ClassSpec { name: "K".encode_utf16().collect(), wndproc: 0x1000, ..ClassSpec::default() };
    let (_, r, _) = user_call(&mut f, &Request::RegisterClass { spec }, 256);
    assert!(matches!(r, Some(Reply::Atom { atom }) if atom >= 0xC000));
    // A reply that does not fit says how long it is.
    let (s, r, n) = user_call(&mut f, &Request::Desktop {}, 2);
    assert_eq!((s, r.is_none()), (STATUS_BUFFER_TOO_SMALL, true));
    assert!(n > 2);
    // Garbage is refused.
    let bad = f.put(&[0xEE]);
    let out = f.alloc(16);
    let len = f.put_u64(0);
    assert_eq!(
        raw(&mut f, Win32kService::Call.number(), &[bad, 1, out, 16, len]).1,
        poly_ntobj::status::STATUS_INVALID_PARAMETER.0
    );
}

#[test]
fn posting_sets_the_threads_registered_event() {
    let mut f = Fake::new();
    let ev = f.put_u64(0);
    assert_eq!(f.call(Service::CreateEvent, &[ev, 0x1F0003, 0, 1, 0]), STATUS_SUCCESS);
    let handle = f.u64_at(ev);
    let (_, r, _) = user_call(&mut f, &Request::RegisterEvent { handle }, 64);
    assert_eq!(r, Some(Reply::Ok {}));
    let (_, r, _) = user_call(&mut f, &Request::RegisterEvent { handle: 0 }, 64);
    assert_eq!(r, Some(Reply::Value { value: handle }));
    let (_, r, _) = user_call(&mut f, &Request::Post { hwnd: 0, msg: 0x401, w: 1, l: 2 }, 64);
    assert_eq!(r, Some(Reply::Ok {}));
    // The event is signalled now: a wait on it does not time out.
    let zero = f.put(&0i64.to_le_bytes());
    assert_eq!(f.call(Service::WaitForSingleObject, &[handle, 0, zero]), STATUS_SUCCESS);
}

#[test]
fn the_win32k_lockdown_refuses_table_one_only() {
    let mut f = Fake::new();
    f.with_state(|_, p, _| p.win32k_disabled = true);
    assert_eq!(raw(&mut f, Win32kService::Call.number(), &[0, 0, 0, 0, 0]), (Outcome::Handled, LOCKDOWN_STATUS.0));
    assert_eq!(raw(&mut f, 0x1FFF, &[]), (Outcome::Handled, LOCKDOWN_STATUS.0));
    let status = f.call(Service::YieldExecution, &[]);
    assert!(status == STATUS_SUCCESS || status == STATUS_NO_YIELD_PERFORMED);
}

#[test]
fn unknown_numbers_are_unimplemented_and_named_when_known() {
    let mut f = Fake::new();
    assert_eq!(
        raw(&mut f, 0x1FFF, &[]),
        (Outcome::Unimplemented { table: "win32k", number: 0xFFF }, STATUS_NOT_IMPLEMENTED)
    );
    assert_eq!(service_name(0), Some("PolyUserCall"));
    assert_eq!(service_name(0xFFF), None);
    assert!(!NtProcess::new(PID).win32k_disabled);
}
