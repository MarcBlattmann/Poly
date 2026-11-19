/* _kernelrpc_mach_vm_allocate_trap, Mach trap 10 (task, address pointer, size, flags): VM_FLAGS_ANYWHERE (1) allocates zero-filled readable and writable memory at an address it chooses (page aligned), size rounded up to whole pages (a size of 1 gives a page); mmap, munmap and mprotect work on it. */
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
    check("KERN_SUCCESS", vm_allocate(task, &addr, ps, VM_FLAGS_ANYWHERE) == KERN_SUCCESS, 0);
    check("page aligned and not 0", addr != 0 && addr % ps == 0, addr % ps);
    int zero = 1;
    for (i64 i = 0; i < ps; i++) zero &= ((char *)addr)[i] == 0;
    check("zero filled", zero, 0);
    ((char *)addr)[0] = 5;
    ((char *)addr)[ps - 1] = 6;
    check("readable and writable", ((char *)addr)[0] == 5 && ((char *)addr)[ps - 1] == 6, 0);
    u64 one = 0;
    check("a size of 1", vm_allocate(task, &one, 1, VM_FLAGS_ANYWHERE) == KERN_SUCCESS, 0);
    ((char *)one)[ps - 1] = 1;
    check("covers a whole page", ((char *)one)[ps - 1] == 1, 0);
    check("mprotect works on it", bsd3(SYS_mprotect, addr, ps, PROT_READ).err == 0, 0);
    check("munmap works on it", bsd2(SYS_munmap, addr, ps).err == 0, 0);
    check("then it is gone: mprotect ENOMEM", bsd3(SYS_mprotect, addr, ps, PROT_READ).err == ENOMEM, 0);
    vm_deallocate(task, one, 1);
    return finish();
}
