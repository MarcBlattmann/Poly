/* _kernelrpc_mach_vm_deallocate_trap, Mach trap 12, errors: an invalid task is MACH_SEND_INVALID_DEST. Values: an address that is not page aligned, a size of 0 and a range that is not mapped (all KERN_SUCCESS for a range that was freed). */
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
    u64 addr = 0;
    vm_allocate(task, &addr, 2 * ps, VM_FLAGS_ANYWHERE);
    check("an invalid task: MACH_SEND_INVALID_DEST", vm_deallocate(12345, addr, ps) == MACH_SEND_INVALID_DEST, 0);
    value("a size of 0 on mapped memory: kern_return", vm_deallocate(task, addr, 0));
    check("the memory is still mapped", bsd3(SYS_mprotect, addr, ps, PROT_READ).err == 0, 0);
    value("an address one byte in, one page: kern_return", vm_deallocate(task, addr + 1, ps));
    value("memory after that: mprotect of the first page errno", bsd3(SYS_mprotect, addr, ps, PROT_READ).err);
    value("memory after that: mprotect of the second page errno", bsd3(SYS_mprotect, addr + ps, ps, PROT_READ).err);
    value("a range that was never mapped: kern_return", vm_deallocate(task, 0x100000, ps));
    vm_deallocate(task, addr, 2 * ps);
    return finish();
}
