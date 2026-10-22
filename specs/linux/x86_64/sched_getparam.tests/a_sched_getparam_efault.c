/* An unwritable param on an existing pid is EFAULT (14): address 1, a
 * kernel address, a non-canonical address, an unmapped page, a read-only
 * page, a PROT_NONE page, and 4 bytes that run into an unmapped page. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_getparam 143

int test_main(void) {
    expect("ptr_1", syscall2(SYS_sched_getparam, 0, 1), -EFAULT);
    expect("ptr_kernel", syscall2(SYS_sched_getparam, 0, 0xffff800000000000UL), -EFAULT);
    expect("ptr_noncanonical", syscall2(SYS_sched_getparam, 0, 0x0000800000000000UL), -EFAULT);
    expect("ptr_kernel_self_pid", syscall2(SYS_sched_getparam, syscall0(SYS_getpid), 0xffffffffff600000UL), -EFAULT);
    char *m = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("unmap_second", syscall2(SYS_munmap, m + A_PAGE, A_PAGE), 0);
    expect("unmapped", syscall2(SYS_sched_getparam, 0, m + A_PAGE), -EFAULT);
    expect("straddles", syscall2(SYS_sched_getparam, 0, m + A_PAGE - 2), -EFAULT);
    expect("last_4_bytes", syscall2(SYS_sched_getparam, 0, m + A_PAGE - 4), 0);
    expect("make_readonly", syscall3(SYS_mprotect, m, A_PAGE, PROT_READ), 0);
    expect("readonly", syscall2(SYS_sched_getparam, 0, m + 16), -EFAULT);
    expect("make_none", syscall3(SYS_mprotect, m, A_PAGE, PROT_NONE), 0);
    expect("prot_none", syscall2(SYS_sched_getparam, 0, m + 16), -EFAULT);
    return finish();
}
