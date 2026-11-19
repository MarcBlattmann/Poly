/* _kernelrpc_mach_port_type_trap, Mach trap 76 (task, name, type pointer): the rights a name holds as a bit set: a receive right 0x20000, a send right 0x10000, both 0x30000; the task's, thread's and host's own names are send rights (0x10000). */
#include "poly_mtest.h"

/* The traps by their kernel names (<mach/mach_traps.h>). Results are kern_return_t (32 bits). */
static __attribute__((unused)) i64 page_size(void) {
    i64 ps = 0;
    size_t len = sizeof ps;
    bsd6(SYS_sysctlbyname, "hw.pagesize", 11, &ps, &len, 0, 0);
    return ps;
}
static __attribute__((unused)) i64 self(void) { return mach0(MACH_task_self_trap); }
static __attribute__((unused)) u32 port_allocate(i64 task, i64 right, u32 *name) { return (u32)poly_mach6(MACH__kernelrpc_mach_port_allocate_trap, task, right, (i64)name, 0, 0, 0); }
static __attribute__((unused)) u32 port_deallocate(i64 task, i64 name) { return (u32)poly_mach6(MACH__kernelrpc_mach_port_deallocate_trap, task, name, 0, 0, 0, 0); }
static __attribute__((unused)) u32 port_mod_refs(i64 task, i64 name, i64 right, i64 delta) { return (u32)poly_mach6(MACH__kernelrpc_mach_port_mod_refs_trap, task, name, right, delta, 0, 0); }
static __attribute__((unused)) u32 port_insert_right(i64 task, i64 name, i64 poly, i64 type) { return (u32)poly_mach6(MACH__kernelrpc_mach_port_insert_right_trap, task, name, poly, type, 0, 0); }
static __attribute__((unused)) u32 port_type(i64 task, i64 name, u32 *type) { return (u32)poly_mach6(MACH__kernelrpc_mach_port_type_trap, task, name, (i64)type, 0, 0, 0); }
static __attribute__((unused)) u32 vm_allocate(i64 task, u64 *addr, i64 size, i64 flags) { return (u32)poly_mach6(MACH__kernelrpc_mach_vm_allocate_trap, task, (i64)addr, size, flags, 0, 0); }
static __attribute__((unused)) u32 vm_deallocate(i64 task, u64 addr, i64 size) { return (u32)poly_mach6(MACH__kernelrpc_mach_vm_deallocate_trap, task, (i64)addr, size, 0, 0, 0); }
static __attribute__((unused)) u32 vm_protect(i64 task, u64 addr, i64 size, i64 set_max, i64 prot) { return (u32)poly_mach6(MACH__kernelrpc_mach_vm_protect_trap, task, (i64)addr, size, set_max, prot, 0); }
/* A receive right made for a test, and its type. */
static __attribute__((unused)) u32 type_of(i64 task, u32 name) { u32 ty = 0; return port_type(task, name, &ty) ? 0xdead0000u : ty; }

int test_main(void) {
    i64 task = self();
    (void)task;
    u32 a = 0, ty = 0;
    port_allocate(task, MACH_PORT_RIGHT_RECEIVE, &a);
    check("a new receive right: 0x20000", type_of(task, a) == MACH_PORT_TYPE_RECEIVE, type_of(task, a));
    port_insert_right(task, a, a, MACH_MSG_TYPE_MAKE_SEND);
    check("with a send right made from it: 0x30000", type_of(task, a) == (MACH_PORT_TYPE_RECEIVE | MACH_PORT_TYPE_SEND), type_of(task, a));
    check("the task's own name: send", type_of(task, task) == MACH_PORT_TYPE_SEND, type_of(task, task));
    check("the thread's own name: send", type_of(task, mach0(MACH_thread_self_trap)) == MACH_PORT_TYPE_SEND, 0);
    check("the host's own name: send", type_of(task, mach0(MACH_host_self_trap)) == MACH_PORT_TYPE_SEND, 0);
    check("the call returns KERN_SUCCESS", port_type(task, a, &ty) == KERN_SUCCESS, 0);
    port_mod_refs(task, a, MACH_PORT_RIGHT_RECEIVE, -1);
    port_deallocate(task, a);
    return finish();
}
