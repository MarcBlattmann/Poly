/* An unreadable param is EFAULT (14). Only the 4-byte sched_priority is
 * read: 4 bytes before an unmapped page work, 2 do not. The pointer is
 * read before the pid is looked up. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

int test_main(void) {
    expect("ptr_1", syscall2(SYS_sched_setparam, 0, 1), -EFAULT);
    expect("ptr_kernel", syscall2(SYS_sched_setparam, 0, 0xffff800000000000UL), -EFAULT);
    expect("ptr_noncanonical", syscall2(SYS_sched_setparam, 0, 0x0000800000000000UL), -EFAULT);
    char *m = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("unmap_second", syscall2(SYS_munmap, m + A_PAGE, A_PAGE), 0);
    *(i32 *)(m + A_PAGE - 4) = 0;
    expect("last_4_bytes", syscall2(SYS_sched_setparam, 0, m + A_PAGE - 4), 0);
    expect("straddles", syscall2(SYS_sched_setparam, 0, m + A_PAGE - 2), -EFAULT);
    expect("unmapped", syscall2(SYS_sched_setparam, 0, m + A_PAGE), -EFAULT);
    expect("missing_pid_bad_ptr", syscall2(SYS_sched_setparam, 0x3fffffff, 1), -EFAULT);
    expect("prot_none_ok_after", syscall3(SYS_mprotect, m, A_PAGE, PROT_NONE), 0);
    expect("prot_none", syscall2(SYS_sched_setparam, 0, m + A_PAGE - 4), -EFAULT);
    return finish();
}
