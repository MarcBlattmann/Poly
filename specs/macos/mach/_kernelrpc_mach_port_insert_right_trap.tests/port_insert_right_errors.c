/* _kernelrpc_mach_port_insert_right_trap, Mach trap 21, errors: an invalid task port is MACH_SEND_INVALID_DEST; a name that is not in the task is KERN_INVALID_NAME (15). Values: a message type that does not exist, a port set as the source. */
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
    u32 a = 0, set = 0;
    port_allocate(task, MACH_PORT_RIGHT_RECEIVE, &a);
    port_allocate(task, MACH_PORT_RIGHT_PORT_SET, &set);
    check("an invalid task: MACH_SEND_INVALID_DEST", port_insert_right(12345, a, a, MACH_MSG_TYPE_MAKE_SEND) == MACH_SEND_INVALID_DEST, 0);
    value("a name that is not in the task as the port: kern_return", port_insert_right(task, a, 0x12345, MACH_MSG_TYPE_MAKE_SEND));
    value("a message type 99: kern_return", port_insert_right(task, a, a, 99));
    value("MAKE_SEND of a port set: kern_return", port_insert_right(task, set, set, MACH_MSG_TYPE_MAKE_SEND));
    check("a good call afterwards", port_insert_right(task, a, a, MACH_MSG_TYPE_MAKE_SEND) == KERN_SUCCESS, 0);
    port_mod_refs(task, a, MACH_PORT_RIGHT_RECEIVE, -1);
    port_deallocate(task, a);
    port_mod_refs(task, set, MACH_PORT_RIGHT_PORT_SET, -1);
    return finish();
}
