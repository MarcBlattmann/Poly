//! [`crate::dispatch_unlocked`] (ADR 0247): the calls the kernel runs without its locks and its
//! end-of-call work give the answers the full dispatcher gives, and every other call is left to it.

use super::*;

fn regs_for(service: Service, args: &[u64]) -> SyscallRegs {
    let mut regs = SyscallRegs { rax: u64::from(service.number()), rsp: STACK, ..Default::default() };
    let slots = [&mut regs.r10, &mut regs.rdx, &mut regs.r8, &mut regs.r9];
    for (slot, &a) in slots.into_iter().zip(args) {
        *slot = a;
    }
    regs
}

#[test]
fn the_performance_counter_is_read_as_the_dispatcher_reads_it() {
    let mut k = Fake::new();
    let (counter, frequency) = (k.put(&[0u8; 8]), k.put(&[0u8; 8]));
    let mut regs = regs_for(Service::QueryPerformanceCounter, &[counter, frequency]);
    assert_eq!(crate::dispatch_unlocked(&mut k, &mut regs), Some(poly_abi::Outcome::Handled));
    assert_eq!(NtStatus(regs.rax as u32), STATUS_SUCCESS);
    let (fast_count, fast_frequency) = (k.u64_at(counter), k.u64_at(frequency));
    // The full path, for comparison.
    let (counter2, frequency2) = (k.put(&[0u8; 8]), k.put(&[0u8; 8]));
    assert_eq!(k.call(Service::QueryPerformanceCounter, &[counter2, frequency2]), STATUS_SUCCESS);
    assert_eq!(fast_frequency, k.u64_at(frequency2));
    assert_ne!(fast_frequency, 0);
    assert!(fast_count <= k.u64_at(counter2), "the counter does not go back");
}

#[test]
fn a_bad_pointer_fails_the_same_way() {
    let mut k = Fake::new();
    let mut regs = regs_for(Service::QueryPerformanceCounter, &[0x10, 0]);
    assert_eq!(crate::dispatch_unlocked(&mut k, &mut regs), Some(poly_abi::Outcome::Handled));
    let fast = regs.rax as u32;
    assert_eq!(k.call(Service::QueryPerformanceCounter, &[0x10, 0]).0, fast);
    assert_ne!(fast, STATUS_SUCCESS.0);
}

#[test]
fn every_other_call_is_left_to_the_dispatcher() {
    let mut k = Fake::new();
    for service in [Service::Close, Service::SetEvent, Service::QuerySystemTime, Service::DelayExecution] {
        let mut regs = regs_for(service, &[0, 0]);
        let before = regs.rax;
        assert_eq!(crate::dispatch_unlocked(&mut k, &mut regs), None, "{}", service.name());
        assert_eq!(regs.rax, before, "untouched");
    }
}
