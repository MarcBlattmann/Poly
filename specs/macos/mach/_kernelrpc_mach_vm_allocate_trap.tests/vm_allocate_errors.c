/* _kernelrpc_mach_vm_allocate_trap, Mach trap 10, errors: a size that cannot fit is KERN_NO_SPACE (3); flags with bits the call does not know are KERN_INVALID_ARGUMENT (4); an address pointer at 8 is KERN_MEMORY_ERROR (10); an invalid task is MACH_SEND_INVALID_DEST. Value: size 0. */
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
    i64 ps = page_size();
    u64 a = 0;
    check("a size of 1 << 60: KERN_NO_SPACE", vm_allocate(task, &a, (i64)1 << 60, VM_FLAGS_ANYWHERE) == KERN_NO_SPACE, 0);
    check("flags 0x7fffffff: KERN_INVALID_ARGUMENT", vm_allocate(task, &a, ps, 0x7fffffff) == KERN_INVALID_ARGUMENT, 0);
    check("an address pointer at 8: KERN_MEMORY_ERROR", vm_allocate(task, (u64 *)8, ps, VM_FLAGS_ANYWHERE) == KERN_MEMORY_ERROR, 0);
    check("an invalid task: MACH_SEND_INVALID_DEST", vm_allocate(12345, &a, ps, VM_FLAGS_ANYWHERE) == MACH_SEND_INVALID_DEST, 0);
    u64 z = 0xabc;
    value("a size of 0: kern_return", vm_allocate(task, &z, 0, VM_FLAGS_ANYWHERE));
    value("the address after a size of 0", (i64)z);
    check("a good call afterwards", vm_allocate(task, &a, ps, VM_FLAGS_ANYWHERE) == KERN_SUCCESS, 0);
    vm_deallocate(task, a, ps);
    return finish();
}
