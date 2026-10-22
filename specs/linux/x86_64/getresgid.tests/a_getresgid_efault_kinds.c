/* getresgid: EFAULT for every kind of address the process cannot write:
 * read-only and inaccessible mappings, an unmapped page, the program's
 * own code, kernel addresses, a non-canonical address and the vsyscall
 * page. The process's group ids are unchanged afterwards. */
#include "a_sys.h"

#define SYS_getresgid 120 /* asm/unistd_64.h */

static i64 one(u64 bad) {
    u32 a, b;
    return syscall3(SYS_getresgid, &a, &b, bad);
}

int test_main(void) {
    u32 r0, e0, s0;
    syscall3(SYS_getresgid, &r0, &e0, &s0);
    i64 ro = syscall6(SYS_mmap, 0, A_PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("read_only_efault", one((u64)ro), -EFAULT);
    check("read_only_page_still_zero", *(volatile u32 *)ro == 0, *(volatile u32 *)ro);
    i64 none = syscall6(SYS_mmap, 0, A_PAGE, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("prot_none_efault", one((u64)none), -EFAULT);
    i64 gone = syscall6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, gone, A_PAGE);
    expect("unmapped_efault", one((u64)gone), -EFAULT);
    expect("own_code_efault", one((u64)&test_main), -EFAULT);
    expect("kernel_address_efault", one(0xffff880000000000UL), -EFAULT);
    expect("top_of_memory_efault", one(0xfffffffffffffffcUL), -EFAULT);
    expect("non_canonical_efault", one(0x0000800000000000UL), -EFAULT);
    expect("vsyscall_page_efault", one(0xffffffffff600000UL), -EFAULT);
    /* After mprotect to writable the same page works. */
    syscall3(SYS_mprotect, ro, A_PAGE, PROT_READ | PROT_WRITE);
    expect("made_writable_ok", one((u64)ro), 0);
    expect("made_writable_value", *(volatile u32 *)ro, s0);
    u32 r, e, s;
    syscall3(SYS_getresgid, &r, &e, &s);
    check("ids_unchanged", r == r0 && e == e0 && s == s0, (i64)r);
    return finish();
}
