/* mach_reply_port, Mach trap 26: a hundred calls give a hundred different names, all receive rights; destroying them leaves the names invalid. */
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
    i64 names[100];
    int distinct = 1, all_receive = 1;
    for (int i = 0; i < 100; i++) {
        names[i] = mach0(MACH_mach_reply_port);
        for (int j = 0; j < i; j++) distinct &= names[i] != names[j];
        all_receive &= type_of(task, (u32)names[i]) == MACH_PORT_TYPE_RECEIVE;
    }
    check("a hundred different names", distinct, 0);
    check("all receive rights", all_receive, 0);
    int destroyed = 1;
    for (int i = 0; i < 100; i++) destroyed &= port_mod_refs(task, names[i], MACH_PORT_RIGHT_RECEIVE, -1) == KERN_SUCCESS;
    check("all destroyed", destroyed, 0);
    u32 ty = 0;
    check("a destroyed name is invalid", port_type(task, names[0], &ty) == KERN_INVALID_NAME, 0);
    return finish();
}
