//! Table 1 (win32k, ADR 0128): every service is reached, answers
//! `STATUS_NOT_IMPLEMENTED` and is reported as unimplemented until its
//! slice; the win32k lockdown refuses all of them; table 0 is untouched.

use super::*;
use crate::STATUS_NOT_IMPLEMENTED;
use crate::win32k::{LOCKDOWN_STATUS, service_name};
use poly_abi::Outcome;
use poly_win32k::services::{WIN32K_SERVICES, Win32kService};

/// Make the system call `number` as a `win32u` stub would.
fn call(f: &mut Fake, number: u32, args: [u64; 4]) -> (Outcome, u32) {
    let mut regs = SyscallRegs {
        rax: u64::from(number),
        rsp: STACK,
        r10: args[0],
        rdx: args[1],
        r8: args[2],
        r9: args[3],
        ..Default::default()
    };
    let outcome = crate::dispatch_with(f, &mut regs);
    (outcome, regs.rax as u32)
}

#[test]
fn every_win32k_service_is_routed_and_not_implemented_yet() {
    let mut f = Fake::new();
    for &(service, number, name) in &WIN32K_SERVICES {
        let (outcome, status) = call(&mut f, number, [1, 2, 3, 4]);
        assert_eq!(status, STATUS_NOT_IMPLEMENTED, "{name}");
        assert_eq!(outcome, Outcome::Unimplemented { table: "win32k", number: u64::from(number & 0xfff) }, "{name}");
        assert_eq!(service_name(u64::from(number & 0xfff)), Some(service.name()));
    }
    // A table-1 number Poly does not know is unimplemented too, unnamed.
    assert_eq!(
        call(&mut f, 0x1FFF, [0; 4]),
        (Outcome::Unimplemented { table: "win32k", number: 0xFFF }, STATUS_NOT_IMPLEMENTED)
    );
    assert_eq!(service_name(0xFFF), None);
}

#[test]
fn the_win32k_lockdown_refuses_table_one_only() {
    let mut f = Fake::new();
    f.with_state(|_, p, _| p.win32k_disabled = true);
    let number = Win32kService::CreateWindow.number();
    assert_eq!(call(&mut f, number, [0; 4]), (Outcome::Handled, LOCKDOWN_STATUS.0));
    assert_eq!(call(&mut f, 0x1FFF, [0; 4]), (Outcome::Handled, LOCKDOWN_STATUS.0));
    // Executive services still work: NtYieldExecution.
    let status = f.call(Service::YieldExecution, &[]);
    assert!(status == STATUS_SUCCESS || status == STATUS_NO_YIELD_PERFORMED);
}

#[test]
fn a_new_system_has_an_empty_desktop() {
    let sys = NtSystem::new();
    assert_eq!(sys.win32k.desktop.surfaces().count(), 0);
    assert!(!sys.win32k.desktop.is_connected());
    assert!(!NtProcess::new(PID).win32k_disabled);
}
