//! Host tests of the dispatcher and every implemented system call, against
//! [`fake::FakeKernel`].

mod fake;
mod files;
mod futex;
mod io;
mod mem;
mod process;
mod signal;
mod startup;
mod task;

use crate::{ENOSYS, Errno, Linux, err, nr};
use fake::{FakeKernel, call, sys};
use poly_abi::{Outcome, SyscallDispatch, SyscallRegs};

#[test]
fn unimplemented_returns_negative_enosys() {
    let mut r = SyscallRegs { rax: 39, ..Default::default() };
    assert_eq!(Linux::dispatch(&mut r), Outcome::Unimplemented { table: "linux", number: 39 });
    assert_eq!(r.rax as i64, -38);
    assert_eq!(err(ENOSYS), Errno::ENOSYS.to_rax());
}

#[test]
fn dispatch_with_kernel_reports_unimplemented_calls() {
    let mut k = FakeKernel::new();
    for number in [nr::SWAPON, nr::RSEQ, 400, 472, 473, 0x4000_0000, u64::MAX] {
        let (outcome, rax) = call(&mut k, number, &[1, 2, 3]);
        assert_eq!(outcome, Outcome::Unimplemented { table: "linux", number });
        assert_eq!(rax as i64, -38);
    }
}

#[test]
fn dispatch_writes_only_rax() {
    let mut k = FakeKernel::new();
    let mut regs =
        SyscallRegs { rax: nr::GETPID, rdi: 1, rsi: 2, rdx: 3, r10: 4, r8: 5, r9: 6, rflags: 0x202, rsp: 0x7fff_0000 };
    let before = regs;
    assert_eq!(Linux::dispatch_with(&mut k, &mut regs), Outcome::Handled);
    assert_eq!(regs, SyscallRegs { rax: 100, ..before });
}

#[test]
fn errors_come_back_as_negative_errno() {
    let mut k = FakeKernel::new();
    let (outcome, rax) = call(&mut k, nr::CLOSE, &[77]);
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(rax as i64, -9);
    assert_eq!(sys(&mut k, nr::CLOSE, &[77]), Err(Errno::EBADF));
}

#[test]
fn every_implemented_call_reports_handled() {
    // Arguments that make each call fail early or do nothing harmful: the
    // point is the outcome, not the result.
    for (number, _) in crate::table::implemented() {
        let mut k = FakeKernel::new();
        let (outcome, _) = call(&mut k, number, &[u64::MAX, 0, 0, 0, u64::MAX, 0]);
        assert_eq!(outcome, Outcome::Handled, "{:?}", crate::table::name(number));
    }
}
