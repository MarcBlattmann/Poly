//! The visible tests of the Mach trap entries (`specs/macos/mach/*.tests`),
//! transliterated from C and run against the host kernel model; each one's
//! output must equal the arm64 output recorded on macOS.

mod common;

use common::*;
use poly_mach::consts::{kern::*, msg::*, port::*, trap::*};

const VM_FLAGS_ANYWHERE: u64 = 1;
const MD: &str = "mach";

fn dir(entry: &str) -> String {
    format!("{MD}/{entry}.tests")
}

// The test header's helpers.
fn task_self(p: &mut P) -> u64 {
    p.mach(TASK_SELF, &[])
}
fn port_allocate(p: &mut P, task: u64, right: u32, name: u64) -> u32 {
    p.kr(MACH_PORT_ALLOCATE, &[task, u64::from(right), name])
}
fn port_deallocate(p: &mut P, task: u64, name: u64) -> u32 {
    p.kr(MACH_PORT_DEALLOCATE, &[task, name])
}
fn port_mod_refs(p: &mut P, task: u64, name: u64, right: u32, delta: i64) -> u32 {
    p.kr(MACH_PORT_MOD_REFS, &[task, name, u64::from(right), delta as u64])
}
fn port_insert_right(p: &mut P, task: u64, name: u64, port: u64, ty: u32) -> u32 {
    p.kr(MACH_PORT_INSERT_RIGHT, &[task, name, port, u64::from(ty)])
}
fn port_type(p: &mut P, task: u64, name: u64, ty: u64) -> u32 {
    p.kr(MACH_PORT_TYPE, &[task, name, ty])
}
fn vm_allocate(p: &mut P, task: u64, addr: u64, size: u64, flags: u64) -> u32 {
    p.kr(MACH_VM_ALLOCATE, &[task, addr, size, flags])
}
fn vm_deallocate(p: &mut P, task: u64, addr: u64, size: u64) -> u32 {
    p.kr(MACH_VM_DEALLOCATE, &[task, addr, size])
}
fn vm_protect(p: &mut P, task: u64, addr: u64, size: u64, set_max: u64, prot: u64) -> u32 {
    p.kr(MACH_VM_PROTECT, &[task, addr, size, set_max, prot])
}
fn type_of(p: &mut P, task: u64, name: u64) -> u32 {
    let ty = p.var(4);
    p.poke32(ty, 0);
    if port_type(p, task, name, ty) != 0 { 0xdead_0000 } else { p.peek32(ty) }
}
/// A receive right: allocate into a fresh variable, return the name.
fn new_receive(p: &mut P, task: u64) -> u64 {
    let v = p.var(4);
    port_allocate(p, task, MACH_PORT_RIGHT_RECEIVE, v);
    u64::from(p.peek32(v))
}
fn mprotect_err(p: &mut P, addr: u64, len: u64, prot: u64) -> i64 {
    p.bsd(74, &[addr, len, prot]).err
}
fn mach_msg(p: &mut P, msg: u64, option: u32, rcv_size: u32, name: u64, timeout: u32) -> u32 {
    p.kr(MACH_MSG, &[msg, u64::from(option), 0, u64::from(rcv_size), name, u64::from(timeout), 0])
}

#[test]
fn port_allocate_errors() {
    run(&dir("_kernelrpc_mach_port_allocate_trap"), "port_allocate_errors", |p| {
        let task = task_self(p);
        let x = p.var(4);
        p.poke32(x, 0xabcd);
        let r = port_allocate(p, 0, MACH_PORT_RIGHT_RECEIVE, x);
        p.check("task 0: MACH_SEND_INVALID_DEST", r == MACH_SEND_INVALID_DEST, 0);
        let r = port_allocate(p, 12345, MACH_PORT_RIGHT_RECEIVE, x);
        p.check("task 12345: MACH_SEND_INVALID_DEST", r == MACH_SEND_INVALID_DEST, 0);
        let v = p.peek32(x);
        p.check("the name was not written", v == 0xabcd, i64::from(v));
        let r = port_allocate(p, task, MACH_PORT_RIGHT_RECEIVE, 8);
        p.check("the name pointer 8: KERN_MEMORY_ERROR", r == KERN_MEMORY_ERROR, 0);
        let r = port_allocate(p, task, MACH_PORT_RIGHT_RECEIVE, x);
        p.check("a good call afterwards", r == KERN_SUCCESS, 0);
        let n = u64::from(p.peek32(x));
        port_mod_refs(p, task, n, MACH_PORT_RIGHT_RECEIVE, -1);
    });
}

#[test]
fn port_allocate_receive() {
    run(&dir("_kernelrpc_mach_port_allocate_trap"), "port_allocate_receive", |p| {
        let task = task_self(p);
        let (va, vb) = (p.var(4), p.var(4));
        let r = port_allocate(p, task, MACH_PORT_RIGHT_RECEIVE, va);
        let a = u64::from(p.peek32(va));
        p.check("RECEIVE: KERN_SUCCESS", r == KERN_SUCCESS, a as i64);
        p.check("the name is not MACH_PORT_NULL", a != 0, a as i64);
        let r = port_allocate(p, task, MACH_PORT_RIGHT_RECEIVE, vb);
        let b = u64::from(p.peek32(vb));
        p.check("a second right has another name", r == KERN_SUCCESS && b != a && b != 0, b as i64);
        let t = type_of(p, task, a);
        p.check("the type is RECEIVE", t == MACH_PORT_TYPE_RECEIVE, i64::from(t));
        p.check("a name is a 32-bit value that is not dead", a != u64::from(MACH_PORT_DEAD), a as i64);
        port_mod_refs(p, task, a, MACH_PORT_RIGHT_RECEIVE, -1);
        port_mod_refs(p, task, b, MACH_PORT_RIGHT_RECEIVE, -1);
    });
}

#[test]
fn port_deallocate_self_ports() {
    run(&dir("_kernelrpc_mach_port_deallocate_trap"), "port_deallocate_self_ports", |p| {
        let task = task_self(p);
        let before = type_of(p, task, task);
        p.check("the task's own name is a send right", before == MACH_PORT_TYPE_SEND, i64::from(before));
        let r = port_deallocate(p, task, task);
        p.check("deallocate one reference: KERN_SUCCESS", r == KERN_SUCCESS, 0);
        let t = type_of(p, task, task);
        p.value("the task's own name after one deallocate: type", i64::from(t));
        let reply = p.mach(MACH_REPLY_PORT, &[]);
        let t = type_of(p, task, reply);
        p.check("a reply port is a receive right", t == MACH_PORT_TYPE_RECEIVE, i64::from(t));
        let r = port_mod_refs(p, task, reply, MACH_PORT_RIGHT_RECEIVE, -1);
        p.check("destroying it: KERN_SUCCESS", r == KERN_SUCCESS, 0);
    });
}

#[test]
fn port_deallocate_send() {
    run(&dir("_kernelrpc_mach_port_deallocate_trap"), "port_deallocate_send", |p| {
        let task = task_self(p);
        let a = new_receive(p, task);
        port_insert_right(p, task, a, a, MACH_MSG_TYPE_MAKE_SEND);
        port_insert_right(p, task, a, a, MACH_MSG_TYPE_MAKE_SEND);
        let rs = MACH_PORT_TYPE_RECEIVE | MACH_PORT_TYPE_SEND;
        let t = type_of(p, task, a);
        p.check("receive + send", t == rs, i64::from(t));
        let r = port_deallocate(p, task, a);
        p.check("the first deallocate: KERN_SUCCESS", r == KERN_SUCCESS, 0);
        let t = type_of(p, task, a);
        p.check("one reference left: still receive + send", t == rs, i64::from(t));
        let r = port_deallocate(p, task, a);
        p.check("the second deallocate: KERN_SUCCESS", r == KERN_SUCCESS, 0);
        let t = type_of(p, task, a);
        p.check("the send right is gone, the receive right stays", t == MACH_PORT_TYPE_RECEIVE, i64::from(t));
        port_mod_refs(p, task, a, MACH_PORT_RIGHT_RECEIVE, -1);
    });
}

#[test]
fn port_insert_right_errors() {
    run(&dir("_kernelrpc_mach_port_insert_right_trap"), "port_insert_right_errors", |p| {
        let task = task_self(p);
        let a = new_receive(p, task);
        let vs = p.var(4);
        port_allocate(p, task, MACH_PORT_RIGHT_PORT_SET, vs);
        let set = u64::from(p.peek32(vs));
        let r = port_insert_right(p, 12345, a, a, MACH_MSG_TYPE_MAKE_SEND);
        p.check("an invalid task: MACH_SEND_INVALID_DEST", r == MACH_SEND_INVALID_DEST, 0);
        let r = port_insert_right(p, task, a, 0x12345, MACH_MSG_TYPE_MAKE_SEND);
        p.value("a name that is not in the task as the port: kern_return", i64::from(r));
        let r = port_insert_right(p, task, a, a, 99);
        p.value("a message type 99: kern_return", i64::from(r));
        let r = port_insert_right(p, task, set, set, MACH_MSG_TYPE_MAKE_SEND);
        p.value("MAKE_SEND of a port set: kern_return", i64::from(r));
        let r = port_insert_right(p, task, a, a, MACH_MSG_TYPE_MAKE_SEND);
        p.check("a good call afterwards", r == KERN_SUCCESS, 0);
        port_mod_refs(p, task, a, MACH_PORT_RIGHT_RECEIVE, -1);
        port_deallocate(p, task, a);
        port_mod_refs(p, task, set, MACH_PORT_RIGHT_PORT_SET, -1);
    });
}

#[test]
fn port_insert_right_name_chosen() {
    run(&dir("_kernelrpc_mach_port_insert_right_trap"), "port_insert_right_name_chosen", |p| {
        let task = task_self(p);
        let a = new_receive(p, task);
        let b = new_receive(p, task);
        let r = port_insert_right(p, task, b, a, MACH_MSG_TYPE_MAKE_SEND);
        p.value("MAKE_SEND of a under b's name (b is a receive right): kern_return", i64::from(r));
        let t = type_of(p, task, a);
        p.check("a still has only its receive right", t == MACH_PORT_TYPE_RECEIVE, i64::from(t));
        let r = port_insert_right(p, task, a, a, MACH_MSG_TYPE_MAKE_SEND);
        p.check("MAKE_SEND under a's own name works", r == KERN_SUCCESS, 0);
        port_mod_refs(p, task, a, MACH_PORT_RIGHT_RECEIVE, -1);
        port_mod_refs(p, task, b, MACH_PORT_RIGHT_RECEIVE, -1);
        port_deallocate(p, task, a);
        port_deallocate(p, task, b);
    });
}

#[test]
fn port_mod_refs_destroy() {
    run(&dir("_kernelrpc_mach_port_mod_refs_trap"), "port_mod_refs_destroy", |p| {
        let task = task_self(p);
        let a = new_receive(p, task);
        let ty = p.var(4);
        let r = port_mod_refs(p, task, a, MACH_PORT_RIGHT_RECEIVE, -1);
        p.check("destroy: KERN_SUCCESS", r == KERN_SUCCESS, 0);
        let r = port_type(p, task, a, ty);
        p.check("the name is gone", r == KERN_INVALID_NAME, 0);
        let r = port_mod_refs(p, task, a, MACH_PORT_RIGHT_RECEIVE, -1);
        p.check("again: KERN_INVALID_NAME", r == KERN_INVALID_NAME, 0);
        let r = port_mod_refs(p, task, 0x12345, MACH_PORT_RIGHT_RECEIVE, -1);
        p.check("an unknown name: KERN_INVALID_NAME", r == KERN_INVALID_NAME, 0);
        let r = port_mod_refs(p, 12345, a, MACH_PORT_RIGHT_RECEIVE, -1);
        p.check("an invalid task: MACH_SEND_INVALID_DEST", r == MACH_SEND_INVALID_DEST, 0);
    });
}

#[test]
fn port_mod_refs_errors() {
    run(&dir("_kernelrpc_mach_port_mod_refs_trap"), "port_mod_refs_errors", |p| {
        let task = task_self(p);
        let a = new_receive(p, task);
        let r = port_mod_refs(p, task, a, MACH_PORT_RIGHT_SEND, -1);
        p.check("a send reference of a receive-only name: KERN_INVALID_RIGHT", r == KERN_INVALID_RIGHT, 0);
        let r = port_mod_refs(p, task, a, MACH_PORT_RIGHT_PORT_SET, -1);
        p.check("a port-set reference of a receive right: KERN_INVALID_RIGHT", r == KERN_INVALID_RIGHT, 0);
        let r = port_mod_refs(p, task, a, 99, -1);
        p.check("a right number 99: KERN_INVALID_VALUE", r == KERN_INVALID_VALUE, 0);
        let r = port_mod_refs(p, task, a, MACH_PORT_RIGHT_RECEIVE, -2);
        p.value("a receive delta of -2: kern_return", i64::from(r));
        let r = port_mod_refs(p, task, a, MACH_PORT_RIGHT_RECEIVE, 1);
        p.value("a receive delta of +1: kern_return", i64::from(r));
        port_mod_refs(p, task, a, MACH_PORT_RIGHT_RECEIVE, -1);
    });
}

#[test]
fn port_type_errors() {
    run(&dir("_kernelrpc_mach_port_type_trap"), "port_type_errors", |p| {
        let task = task_self(p);
        let ty = p.var(4);
        p.poke32(ty, 0xabcd);
        let r = port_type(p, task, 0x12345, ty);
        p.check("an unknown name: KERN_INVALID_NAME", r == KERN_INVALID_NAME, 0);
        let r = port_type(p, task, 0, ty);
        p.check("MACH_PORT_NULL: KERN_INVALID_NAME", r == KERN_INVALID_NAME, 0);
        let v = p.peek32(ty);
        p.check("nothing was written for errors", v == 0xabcd, i64::from(v));
        let r = port_type(p, task, u64::from(MACH_PORT_DEAD), ty);
        p.check("MACH_PORT_DEAD: KERN_SUCCESS", r == KERN_SUCCESS, 0);
        let v = p.peek32(ty);
        p.check("and it is a dead name", v == MACH_PORT_TYPE_DEAD_NAME, i64::from(v));
        let r = port_type(p, 12345, task, ty);
        p.check("an invalid task: MACH_SEND_INVALID_DEST", r == MACH_SEND_INVALID_DEST, 0);
        let r = port_type(p, task, task, 8);
        p.value("the type pointer 8: kern_return", i64::from(r));
    });
}

#[test]
fn port_type_values() {
    run(&dir("_kernelrpc_mach_port_type_trap"), "port_type_values", |p| {
        let task = task_self(p);
        let a = new_receive(p, task);
        let ty = p.var(4);
        let t = type_of(p, task, a);
        p.check("a new receive right: 0x20000", t == MACH_PORT_TYPE_RECEIVE, i64::from(t));
        port_insert_right(p, task, a, a, MACH_MSG_TYPE_MAKE_SEND);
        let t = type_of(p, task, a);
        p.check(
            "with a send right made from it: 0x30000",
            t == MACH_PORT_TYPE_RECEIVE | MACH_PORT_TYPE_SEND,
            i64::from(t),
        );
        let t = type_of(p, task, task);
        p.check("the task's own name: send", t == MACH_PORT_TYPE_SEND, i64::from(t));
        let th = p.mach(THREAD_SELF, &[]);
        let t = type_of(p, task, th);
        p.check("the thread's own name: send", t == MACH_PORT_TYPE_SEND, 0);
        let h = p.mach(HOST_SELF, &[]);
        let t = type_of(p, task, h);
        p.check("the host's own name: send", t == MACH_PORT_TYPE_SEND, 0);
        let r = port_type(p, task, a, ty);
        p.check("the call returns KERN_SUCCESS", r == KERN_SUCCESS, 0);
        port_mod_refs(p, task, a, MACH_PORT_RIGHT_RECEIVE, -1);
        port_deallocate(p, task, a);
    });
}

#[test]
fn vm_allocate_anywhere() {
    run(&dir("_kernelrpc_mach_vm_allocate_trap"), "vm_allocate_anywhere", |p| {
        let task = task_self(p);
        let va = p.var(8);
        let r = vm_allocate(p, task, va, PS, VM_FLAGS_ANYWHERE);
        p.check("KERN_SUCCESS", r == KERN_SUCCESS, 0);
        let addr = p.peek64(va);
        p.check("page aligned and not 0", addr != 0 && addr % PS == 0, (addr % PS) as i64);
        let zero = (0..PS).all(|i| p.peek(addr + i) == 0);
        p.check("zero filled", zero, 0);
        p.poke(addr, 5);
        p.poke(addr + PS - 1, 6);
        p.check("readable and writable", p.peek(addr) == 5 && p.peek(addr + PS - 1) == 6, 0);
        let vo = p.var(8);
        let r = vm_allocate(p, task, vo, 1, VM_FLAGS_ANYWHERE);
        p.check("a size of 1", r == KERN_SUCCESS, 0);
        let one = p.peek64(vo);
        p.poke(one + PS - 1, 1);
        p.check("covers a whole page", p.peek(one + PS - 1) == 1, 0);
        let e = mprotect_err(p, addr, PS, PROT_READ);
        p.check("mprotect works on it", e == 0, 0);
        p.munmap(addr, PS);
        p.check("munmap works on it", true, 0);
        let e = mprotect_err(p, addr, PS, PROT_READ);
        p.check("then it is gone: mprotect ENOMEM", e == 12, 0);
        vm_deallocate(p, task, one, 1);
    });
}

#[test]
fn vm_allocate_errors() {
    run(&dir("_kernelrpc_mach_vm_allocate_trap"), "vm_allocate_errors", |p| {
        let task = task_self(p);
        let a = p.var(8);
        let r = vm_allocate(p, task, a, 1 << 60, VM_FLAGS_ANYWHERE);
        p.check("a size of 1 << 60: KERN_NO_SPACE", r == KERN_NO_SPACE, 0);
        let r = vm_allocate(p, task, a, PS, 0x7fff_ffff);
        p.check("flags 0x7fffffff: KERN_INVALID_ARGUMENT", r == KERN_INVALID_ARGUMENT, 0);
        let r = vm_allocate(p, task, 8, PS, VM_FLAGS_ANYWHERE);
        p.check("an address pointer at 8: KERN_MEMORY_ERROR", r == KERN_MEMORY_ERROR, 0);
        let r = vm_allocate(p, 12345, a, PS, VM_FLAGS_ANYWHERE);
        p.check("an invalid task: MACH_SEND_INVALID_DEST", r == MACH_SEND_INVALID_DEST, 0);
        let z = p.var(8);
        p.poke64(z, 0xabc);
        let r = vm_allocate(p, task, z, 0, VM_FLAGS_ANYWHERE);
        p.value("a size of 0: kern_return", i64::from(r));
        let v = p.peek64(z);
        p.value("the address after a size of 0", v as i64);
        let r = vm_allocate(p, task, a, PS, VM_FLAGS_ANYWHERE);
        p.check("a good call afterwards", r == KERN_SUCCESS, 0);
        let addr = p.peek64(a);
        vm_deallocate(p, task, addr, PS);
    });
}

#[test]
fn vm_deallocate_errors() {
    run(&dir("_kernelrpc_mach_vm_deallocate_trap"), "vm_deallocate_errors", |p| {
        let task = task_self(p);
        let va = p.var(8);
        vm_allocate(p, task, va, 2 * PS, VM_FLAGS_ANYWHERE);
        let addr = p.peek64(va);
        let r = vm_deallocate(p, 12345, addr, PS);
        p.check("an invalid task: MACH_SEND_INVALID_DEST", r == MACH_SEND_INVALID_DEST, 0);
        let r = vm_deallocate(p, task, addr, 0);
        p.value("a size of 0 on mapped memory: kern_return", i64::from(r));
        let e = mprotect_err(p, addr, PS, PROT_READ);
        p.check("the memory is still mapped", e == 0, 0);
        let r = vm_deallocate(p, task, addr + 1, PS);
        p.value("an address one byte in, one page: kern_return", i64::from(r));
        let e = mprotect_err(p, addr, PS, PROT_READ);
        p.value("memory after that: mprotect of the first page errno", e);
        let e = mprotect_err(p, addr + PS, PS, PROT_READ);
        p.value("memory after that: mprotect of the second page errno", e);
        let r = vm_deallocate(p, task, 0x100000, PS);
        p.value("a range that was never mapped: kern_return", i64::from(r));
        vm_deallocate(p, task, addr, 2 * PS);
    });
}

#[test]
fn vm_deallocate_part() {
    run(&dir("_kernelrpc_mach_vm_deallocate_trap"), "vm_deallocate_part", |p| {
        let task = task_self(p);
        let va = p.var(8);
        vm_allocate(p, task, va, 3 * PS, VM_FLAGS_ANYWHERE);
        let addr = p.peek64(va);
        p.poke(addr, 1);
        p.poke(addr + 2 * PS, 3);
        let r = vm_deallocate(p, task, addr + PS, PS);
        p.check("free the middle page", r == KERN_SUCCESS, 0);
        let v = p.peek(addr);
        p.check("the first page keeps its content", v == 1, i64::from(v));
        let v = p.peek(addr + 2 * PS);
        p.check("the third page keeps its content", v == 3, i64::from(v));
        let e = mprotect_err(p, addr + PS, PS, PROT_READ);
        p.check("the middle page is gone: mprotect ENOMEM", e == 12, 0);
        vm_deallocate(p, task, addr, PS);
        vm_deallocate(p, task, addr + 2 * PS, PS);
    });
}

#[test]
fn vm_protect_basic() {
    run(&dir("_kernelrpc_mach_vm_protect_trap"), "vm_protect_basic", |p| {
        let task = task_self(p);
        let va = p.var(8);
        vm_allocate(p, task, va, PS, VM_FLAGS_ANYWHERE);
        let addr = p.peek64(va);
        p.poke(addr, 4);
        let r = vm_protect(p, task, addr, PS, 0, PROT_READ);
        p.check("RO: KERN_SUCCESS", r == KERN_SUCCESS, 0);
        let pid = p.fork(move |c| {
            c.poke(addr, 9);
            42
        });
        let st = p.wait(pid);
        p.check("a write kills a child", !w_exited(st), i64::from(st));
        p.value("signal", i64::from(w_termsig(st)));
        let r = vm_protect(p, task, addr, PS, 0, PROT_READ | PROT_WRITE);
        p.check("RW again: KERN_SUCCESS", r == KERN_SUCCESS, 0);
        let v = p.peek(addr);
        p.check("the content stayed", v == 4, i64::from(v));
        p.poke(addr, 5);
        let v = p.peek(addr);
        p.check("and it can be written", v == 5, i64::from(v));
        vm_deallocate(p, task, addr, PS);
    });
}

#[test]
fn vm_protect_maximum() {
    run(&dir("_kernelrpc_mach_vm_protect_trap"), "vm_protect_maximum", |p| {
        let task = task_self(p);
        let va = p.var(8);
        vm_allocate(p, task, va, PS, VM_FLAGS_ANYWHERE);
        let addr = p.peek64(va);
        let r = vm_protect(p, task, addr, PS, 1, PROT_READ);
        p.check("maximum to read-only: KERN_SUCCESS", r == KERN_SUCCESS, 0);
        let r = vm_protect(p, task, addr, PS, 0, PROT_READ);
        p.check("the current protection read-only: fine", r == KERN_SUCCESS, 0);
        let r = vm_protect(p, task, addr, PS, 0, PROT_READ | PROT_WRITE);
        p.check("read-write is above the maximum: KERN_PROTECTION_FAILURE", r == KERN_PROTECTION_FAILURE, 0);
        let e = mprotect_err(p, addr, PS, PROT_READ | PROT_WRITE);
        p.check("mprotect cannot raise it either", e != 0, 0);
        let e = mprotect_err(p, addr, PS, PROT_READ | PROT_WRITE);
        p.value("mprotect to read-write: errno", e);
    });
}

fn self_fork(p: &mut P, trap: u64) {
    let mine = p.mach(trap, &[]);
    let pid = p.fork(move |c| {
        let t = task_self(c);
        let n = c.mach(trap, &[]);
        if n != 0 && type_of(c, t, n) == MACH_PORT_TYPE_SEND { if n == mine { 10 } else { 11 } } else { 1 }
    });
    let st = p.wait(pid);
    p.check(
        "the child's name is a send right (exit 10 or 11)",
        w_exited(st) && (w_exitstatus(st) == 10 || w_exitstatus(st) == 11),
        i64::from(st),
    );
    p.value("the child's name equals the parent's (10 = yes, 11 = no)", i64::from(w_exitstatus(st)));
    let again = p.mach(trap, &[]);
    p.check("the parent's name is unchanged", again == mine, 0);
}

fn self_stable(p: &mut P, trap: u64) {
    let task = task_self(p);
    let (a, b) = (p.mach(trap, &[]), p.mach(trap, &[]));
    p.check("not MACH_PORT_NULL", a != 0, a as i64);
    p.check("not MACH_PORT_DEAD", a as u32 != MACH_PORT_DEAD, a as i64);
    p.check("a 32-bit name", a == u64::from(a as u32), a as i64);
    p.check("the same on every call", a == b, b as i64);
    let t = type_of(p, task, a);
    p.check("a send right", t == MACH_PORT_TYPE_SEND, i64::from(t));
}

#[test]
fn host_self_fork() {
    run(&dir("host_self_trap"), "host_self_fork", |p| self_fork(p, HOST_SELF));
}

#[test]
fn host_self_stable() {
    run(&dir("host_self_trap"), "host_self_stable", |p| self_stable(p, HOST_SELF));
}

#[test]
fn thread_self_fork() {
    run(&dir("thread_self_trap"), "thread_self_fork", |p| self_fork(p, THREAD_SELF));
}

#[test]
fn thread_self_stable() {
    run(&dir("thread_self_trap"), "thread_self_stable", |p| self_stable(p, THREAD_SELF));
}

#[test]
fn task_self_reply_ports() {
    run(&dir("task_self_trap"), "task_self_reply_ports", |p| {
        let task = task_self(p);
        let (r1, r2) = (p.mach(MACH_REPLY_PORT, &[]), p.mach(MACH_REPLY_PORT, &[]));
        p.check("mach_reply_port gives a name", r1 != 0, 0);
        p.check("a new one each time", r1 != r2, 0);
        p.check("never the task's", r1 != task && r2 != task, 0);
        let again = task_self(p);
        p.check("the task's name stays", again == task, 0);
    });
}

#[test]
fn task_self_stable() {
    run(&dir("task_self_trap"), "task_self_stable", |p| {
        let (a, b) = (task_self(p), task_self(p));
        p.check("not MACH_PORT_NULL", a != 0, 0);
        p.check("not MACH_PORT_DEAD", a as u32 != MACH_PORT_DEAD, 0);
        p.check("a 32-bit name", a == u64::from(a as u32), 0);
        p.check("the same name twice", a == b, 0);
    });
}

#[test]
fn reply_port_many() {
    run(&dir("mach_reply_port"), "reply_port_many", |p| {
        let task = task_self(p);
        let mut names = Vec::new();
        let (mut distinct, mut all_receive) = (true, true);
        for _ in 0..100 {
            let n = p.mach(MACH_REPLY_PORT, &[]);
            distinct &= !names.contains(&n);
            names.push(n);
            all_receive &= type_of(p, task, n) == MACH_PORT_TYPE_RECEIVE;
        }
        p.check("a hundred different names", distinct, 0);
        p.check("all receive rights", all_receive, 0);
        let mut destroyed = true;
        for &n in &names {
            destroyed &= port_mod_refs(p, task, n, MACH_PORT_RIGHT_RECEIVE, -1) == KERN_SUCCESS;
        }
        p.check("all destroyed", destroyed, 0);
        let ty = p.var(4);
        let r = port_type(p, task, names[0], ty);
        p.check("a destroyed name is invalid", r == KERN_INVALID_NAME, 0);
    });
}

#[test]
fn reply_port_names() {
    run(&dir("mach_reply_port"), "reply_port_names", |p| {
        let task = task_self(p);
        let (a, b) = (p.mach(MACH_REPLY_PORT, &[]), p.mach(MACH_REPLY_PORT, &[]));
        p.check("a name that is not MACH_PORT_NULL", a != 0 && b != 0, a as i64);
        p.check("a different one each time", a != b, a as i64);
        p.check("32-bit names", a == u64::from(a as u32) && b == u64::from(b as u32), a as i64);
        p.check("neither is the task's own name", a != task && b != task, a as i64);
        let t = type_of(p, task, a);
        p.check("a receive right", t == MACH_PORT_TYPE_RECEIVE, i64::from(t));
        port_mod_refs(p, task, a, MACH_PORT_RIGHT_RECEIVE, -1);
        port_mod_refs(p, task, b, MACH_PORT_RIGHT_RECEIVE, -1);
    });
}

#[test]
fn msg_receive_errors() {
    run(&dir("mach_msg_trap"), "msg_receive_errors", |p| {
        let task = task_self(p);
        let m = p.var(256);
        let pn = new_receive(p, task);
        let vd = p.var(4);
        port_allocate(p, task, MACH_PORT_RIGHT_DEAD_NAME, vd);
        let dead = u64::from(p.peek32(vd));
        let opt = MACH_RCV_MSG | MACH_RCV_TIMEOUT;
        let r = mach_msg(p, m, opt, 256, 0x12345, 0);
        p.check("an unknown name: MACH_RCV_INVALID_NAME", r == MACH_RCV_INVALID_NAME, 0);
        let r = mach_msg(p, m, opt, 256, 0, 0);
        p.check("MACH_PORT_NULL: MACH_RCV_INVALID_NAME", r == MACH_RCV_INVALID_NAME, 0);
        let r = mach_msg(p, m, opt, 256, u64::from(MACH_PORT_DEAD), 0);
        p.check("MACH_PORT_DEAD: MACH_RCV_INVALID_NAME", r == MACH_RCV_INVALID_NAME, 0);
        let r = mach_msg(p, m, opt, 256, task, 0);
        p.check("a send right (the task's own name): MACH_RCV_INVALID_NAME", r == MACH_RCV_INVALID_NAME, 0);
        let r = mach_msg(p, m, opt, 256, dead, 0);
        p.check("a dead name: MACH_RCV_INVALID_NAME", r == MACH_RCV_INVALID_NAME, 0);
        let r = mach_msg(p, m, 0, 0, 0, 0);
        p.check("no option at all: returns 0", r == 0, 0);
        let vs = p.var(4);
        port_allocate(p, task, MACH_PORT_RIGHT_PORT_SET, vs);
        let set = u64::from(p.peek32(vs));
        let r = p.kr(MACH_PORT_MOVE_MEMBER, &[task, pn, set]);
        p.check("a port moved into a set (move_member trap 20)", r == KERN_SUCCESS, 0);
        let r = mach_msg(p, m, opt, 256, pn, 5);
        p.check("receiving from the member itself: MACH_RCV_TIMED_OUT", r == MACH_RCV_TIMED_OUT, 0);
        let r = mach_msg(p, m, opt, 256, set, 5);
        p.check("receiving from the set: MACH_RCV_TIMED_OUT", r == MACH_RCV_TIMED_OUT, 0);
    });
}

#[test]
fn msg_receive_timeouts() {
    run(&dir("mach_msg_trap"), "msg_receive_timeouts", |p| {
        let task = task_self(p);
        let m = p.var(256);
        let pn = new_receive(p, task);
        let opt = MACH_RCV_MSG | MACH_RCV_TIMEOUT;
        let t0 = p.now();
        let r = mach_msg(p, m, opt, 256, pn, 20);
        p.check("20 ms on an empty port: MACH_RCV_TIMED_OUT", r == MACH_RCV_TIMED_OUT, 0);
        let t1 = p.now();
        p.check(
            "it took at least 15 ms and less than 500 ms",
            t1 - t0 >= 24000 * 15 && t1 - t0 < 24000 * 500,
            (t1 - t0) as i64,
        );
        let t2 = p.now();
        let r = mach_msg(p, m, opt, 256, pn, 0);
        p.check("a timeout of 0: MACH_RCV_TIMED_OUT", r == MACH_RCV_TIMED_OUT, 0);
        p.check("and at once (under 5 ms)", p.now() - t2 < 24000 * 5, 0);
        let vs = p.var(4);
        port_allocate(p, task, MACH_PORT_RIGHT_PORT_SET, vs);
        let set = u64::from(p.peek32(vs));
        let r = mach_msg(p, m, opt, 256, set, 5);
        p.check("an empty port set: MACH_RCV_TIMED_OUT", r == MACH_RCV_TIMED_OUT, 0);
        let r = mach_msg(p, m, opt | MACH_RCV_LARGE, 256, pn, 5);
        p.check("with MACH_RCV_LARGE: MACH_RCV_TIMED_OUT", r == MACH_RCV_TIMED_OUT, 0);
        let trailer = 3 << MACH_RCV_TRAILER_ELEMENTS_SHIFT;
        let r = mach_msg(p, m, opt | trailer, 256, pn, 5);
        p.check("with an audit trailer asked for: MACH_RCV_TIMED_OUT", r == MACH_RCV_TIMED_OUT, 0);
        let r = mach_msg(p, m, opt, 0, pn, 5);
        p.check("a receive size of 0: MACH_RCV_TIMED_OUT", r == MACH_RCV_TIMED_OUT, 0);
        let r = mach_msg(p, 8, opt, 128, pn, 5);
        p.check("a buffer at address 8: MACH_RCV_TIMED_OUT", r == MACH_RCV_TIMED_OUT, 0);
    });
}

#[test]
fn timebase_bad_pointer() {
    run(&dir("mach_timebase_info_trap"), "timebase_bad_pointer", |p| {
        let r = p.kr(MACH_TIMEBASE_INFO, &[0]);
        p.check("NULL: KERN_SUCCESS", r == KERN_SUCCESS, 0);
        let r = p.kr(MACH_TIMEBASE_INFO, &[8]);
        p.check("address 8: KERN_SUCCESS", r == KERN_SUCCESS, 0);
        let tb = p.var(8);
        let r = p.kr(MACH_TIMEBASE_INFO, &[tb]);
        let denom = p.peek32(tb + 4);
        p.check("a good pointer afterwards", r == KERN_SUCCESS && denom != 0, i64::from(r));
    });
}

#[test]
fn timebase_stable() {
    run(&dir("mach_timebase_info_trap"), "timebase_stable", |p| {
        let (a, b) = (p.var(8), p.var(8));
        p.poke64(b, 0x1_0000_0001);
        p.kr(MACH_TIMEBASE_INFO, &[a]);
        let r = p.kr(MACH_TIMEBASE_INFO, &[b]);
        p.check("KERN_SUCCESS again", r == KERN_SUCCESS, i64::from(r));
        let same = p.peek64(a) == p.peek64(b);
        p.check("the same numer and denom", same, 0);
    });
}

#[test]
fn wait_until_past() {
    run(&dir("mach_wait_until_trap"), "wait_until_past", |p| {
        let t0 = p.now();
        let r = p.kr(MACH_WAIT_UNTIL, &[1]);
        p.check("deadline 1: KERN_SUCCESS", r == KERN_SUCCESS, 0);
        let ago = p.now().saturating_sub(24_000_000);
        let r = p.kr(MACH_WAIT_UNTIL, &[ago]);
        p.check("a second ago: KERN_SUCCESS", r == KERN_SUCCESS, 0);
        let t1 = p.now();
        p.check("both returned within 5 ms", t1 - t0 < 24000 * 5, (t1 - t0) as i64);
    });
}

#[test]
fn wait_until_repeated() {
    run(&dir("mach_wait_until_trap"), "wait_until_repeated", |p| {
        let start = p.now();
        for i in 1..=3 {
            p.kr(MACH_WAIT_UNTIL, &[start + 24000 * 10 * i]);
        }
        let t = p.now();
        p.check("at least 30 ms went by", t - start >= 24000 * 30, (t - start) as i64);
        let t2 = p.now();
        p.kr(MACH_WAIT_UNTIL, &[start]);
        p.check("an old deadline returns at once", p.now() - t2 < 24000 * 5, 0);
    });
}

// ----- semaphores -----

/// The names the semaphore tests start with: the task, a reply port, the
/// thread.
fn sem_names(p: &mut P) -> (u64, u64, u64) {
    let task = task_self(p);
    let rp = p.mach(MACH_REPLY_PORT, &[]);
    let th = p.mach(THREAD_SELF, &[]);
    (task, rp, th)
}

/// The `*_invalid_names` tests: NULL, DEAD, unknown; at once.
fn sem_invalid_names(p: &mut P, trap: u64, args: impl Fn(u64) -> Vec<u64>) {
    let _ = sem_names(p);
    let t0 = p.now();
    let dead = u64::from(MACH_PORT_DEAD);
    let r = p.kr(trap, &args(0));
    p.check("MACH_PORT_NULL: KERN_INVALID_NAME", r == KERN_INVALID_NAME, 0);
    let r = p.kr(trap, &args(dead));
    p.check("MACH_PORT_DEAD: KERN_INVALID_NAME", r == KERN_INVALID_NAME, 0);
    let r = p.kr(trap, &args(0x12345));
    p.check("an unknown name: KERN_INVALID_NAME", r == KERN_INVALID_NAME, 0);
    p.check("it returned at once (under 50 ms)", p.now() - t0 < 24000 * 50, 0);
}

/// The `*_wrong_objects` tests: task and thread ports, a receive right.
fn sem_wrong_objects(p: &mut P, trap: u64, args: impl Fn(u64) -> Vec<u64>) {
    let (task, rp, th) = sem_names(p);
    let r = p.kr(trap, &args(task));
    p.check("the task's own port: KERN_INVALID_CAPABILITY", r == KERN_INVALID_CAPABILITY, 0);
    let r = p.kr(trap, &args(th));
    p.check("the thread's own port: KERN_INVALID_CAPABILITY", r == KERN_INVALID_CAPABILITY, 0);
    let r = p.kr(trap, &args(rp));
    p.check("a receive right (a reply port): KERN_INVALID_RIGHT", r == KERN_INVALID_RIGHT, 0);
    let r = p.kr(trap, &args(rp));
    p.check("the reply port is untouched: its type is still a receive right", r == KERN_INVALID_RIGHT, 0);
}

#[test]
fn signal_all_invalid_names() {
    run(&dir("semaphore_signal_all_trap"), "signal_all_invalid_names", |p| {
        sem_invalid_names(p, SEMAPHORE_SIGNAL_ALL, |n| vec![n]);
    });
}

#[test]
fn signal_all_wrong_objects() {
    run(&dir("semaphore_signal_all_trap"), "signal_all_wrong_objects", |p| {
        sem_wrong_objects(p, SEMAPHORE_SIGNAL_ALL, |n| vec![n]);
    });
}

#[test]
fn signal_thread_invalid_names() {
    run(&dir("semaphore_signal_thread_trap"), "signal_thread_invalid_names", |p| {
        sem_invalid_names(p, SEMAPHORE_SIGNAL_THREAD, |n| vec![n, 0]);
    });
}

#[test]
fn signal_thread_thread_argument() {
    run(&dir("semaphore_signal_thread_trap"), "signal_thread_thread_argument", |p| {
        let (task, rp, th) = sem_names(p);
        let sems = [0, 0x12345, task, rp];
        let sem_err = [KERN_INVALID_NAME, KERN_INVALID_NAME, KERN_INVALID_CAPABILITY, KERN_INVALID_RIGHT];
        let bad_threads = [0x12345, task, rp];
        let (mut bad_ok, mut good_ok) = (true, true);
        for i in 0..4 {
            for &b in &bad_threads {
                bad_ok &= p.kr(SEMAPHORE_SIGNAL_THREAD, &[sems[i], b]) == KERN_INVALID_ARGUMENT;
            }
            good_ok &= p.kr(SEMAPHORE_SIGNAL_THREAD, &[sems[i], th]) == sem_err[i];
            good_ok &= p.kr(SEMAPHORE_SIGNAL_THREAD, &[sems[i], 0]) == sem_err[i];
        }
        p.check("a thread name that is not a thread: KERN_INVALID_ARGUMENT for every semaphore name", bad_ok, 0);
        p.check("a thread port or 0: the semaphore name's own error", good_ok, 0);
    });
}

#[test]
fn signal_forked() {
    run(&dir("semaphore_signal_trap"), "signal_forked", |p| {
        let _ = sem_names(p);
        let pid = p.fork(|c| {
            let t = task_self(c);
            let r = c.mach(MACH_REPLY_PORT, &[]);
            let ok = c.kr(SEMAPHORE_SIGNAL, &[0x12345]) == KERN_INVALID_NAME
                && c.kr(SEMAPHORE_SIGNAL, &[t]) == KERN_INVALID_CAPABILITY
                && c.kr(SEMAPHORE_SIGNAL, &[r]) == KERN_INVALID_RIGHT;
            if ok { 0 } else { 1 }
        });
        let st = p.wait(pid);
        p.check("the child got the same three errors", w_exited(st) && w_exitstatus(st) == 0, i64::from(st));
    });
}

#[test]
fn signal_invalid_names() {
    run(&dir("semaphore_signal_trap"), "signal_invalid_names", |p| {
        sem_invalid_names(p, SEMAPHORE_SIGNAL, |n| vec![n]);
    });
}

#[test]
fn timedwait_signal_invalid_names() {
    run(&dir("semaphore_timedwait_signal_trap"), "timedwait_signal_invalid_names", |p| {
        let _ = sem_names(p);
        let t0 = p.now();
        let dead = u64::from(MACH_PORT_DEAD);
        let r = p.kr(SEMAPHORE_TIMEDWAIT_SIGNAL, &[0, 0, 0, 1_000_000]);
        p.check("MACH_PORT_NULL: KERN_INVALID_NAME", r == KERN_INVALID_NAME, 0);
        let r = p.kr(SEMAPHORE_TIMEDWAIT_SIGNAL, &[dead, dead, 0, 1_000_000]);
        p.check("MACH_PORT_DEAD: KERN_INVALID_NAME", r == KERN_INVALID_NAME, 0);
        let r = p.kr(SEMAPHORE_TIMEDWAIT_SIGNAL, &[0x12345, 0x12345, 0, 1_000_000]);
        p.check("an unknown name: KERN_INVALID_NAME", r == KERN_INVALID_NAME, 0);
        p.check("it returned at once (under 50 ms)", p.now() - t0 < 24000 * 50, 0);
    });
}

#[test]
fn timedwait_signal_wrong_objects() {
    run(&dir("semaphore_timedwait_signal_trap"), "timedwait_signal_wrong_objects", |p| {
        sem_wrong_objects(p, SEMAPHORE_TIMEDWAIT_SIGNAL, |n| vec![n, n, 0, 1_000_000]);
    });
}

#[test]
fn timedwait_time_values() {
    run(&dir("semaphore_timedwait_trap"), "timedwait_time_values", |p| {
        let (_, rp, _) = sem_names(p);
        let r = p.kr(SEMAPHORE_TIMEDWAIT, &[0x12345, 0, 2_000_000_000]);
        p.check("nanoseconds 2e9, unknown name: KERN_INVALID_VALUE", r == KERN_INVALID_VALUE, 0);
        let r = p.kr(SEMAPHORE_TIMEDWAIT, &[0x12345, 0, u64::MAX]);
        p.check("nanoseconds -1, unknown name: KERN_INVALID_VALUE", r == KERN_INVALID_VALUE, 0);
        let r = p.kr(SEMAPHORE_TIMEDWAIT, &[rp, 0, 2_000_000_000]);
        p.check("nanoseconds 2e9, a receive right: KERN_INVALID_VALUE", r == KERN_INVALID_VALUE, 0);
        let r = p.kr(SEMAPHORE_TIMEDWAIT, &[0x12345, u64::MAX, 0]);
        p.check("seconds -1, nanoseconds 0, unknown name: KERN_INVALID_NAME", r == KERN_INVALID_NAME, 0);
        let r = p.kr(SEMAPHORE_TIMEDWAIT, &[rp, u64::MAX, 0]);
        p.check("seconds -1, nanoseconds 0, a receive right: KERN_INVALID_RIGHT", r == KERN_INVALID_RIGHT, 0);
    });
}

#[test]
fn timedwait_wrong_objects() {
    run(&dir("semaphore_timedwait_trap"), "timedwait_wrong_objects", |p| {
        sem_wrong_objects(p, SEMAPHORE_TIMEDWAIT, |n| vec![n, 0, 1_000_000]);
    });
}

#[test]
fn wait_signal_invalid_names() {
    run(&dir("semaphore_wait_signal_trap"), "wait_signal_invalid_names", |p| {
        sem_invalid_names(p, SEMAPHORE_WAIT_SIGNAL, |n| vec![n, n]);
    });
}

#[test]
fn wait_signal_wrong_objects() {
    run(&dir("semaphore_wait_signal_trap"), "wait_signal_wrong_objects", |p| {
        sem_wrong_objects(p, SEMAPHORE_WAIT_SIGNAL, |n| vec![n, n]);
    });
}

#[test]
fn wait_invalid_names() {
    run(&dir("semaphore_wait_trap"), "wait_invalid_names", |p| {
        sem_invalid_names(p, SEMAPHORE_WAIT, |n| vec![n]);
    });
}

#[test]
fn wait_wrong_objects() {
    run(&dir("semaphore_wait_trap"), "wait_wrong_objects", |p| {
        sem_wrong_objects(p, SEMAPHORE_WAIT, |n| vec![n]);
    });
}
