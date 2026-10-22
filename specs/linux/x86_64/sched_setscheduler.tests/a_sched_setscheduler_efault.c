/* param must be readable: only its 4-byte sched_priority is read, so a
 * param in the last 4 bytes of a page before an unmapped page works.
 * Address 1, a kernel address, a non-canonical address, a PROT_NONE page
 * and 4 bytes straddling into an unmapped page are EFAULT (14), and the
 * policy is not changed. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_BATCH 3
#define SCHED_IDLE 5
#define SCHED_DEADLINE 6
#define SCHED_RESET_ON_FORK 0x40000000

static A_UNUSED i64 setsch(i64 pid, i64 policy, i32 prio) {
    i32 p = prio;
    return syscall3(SYS_sched_setscheduler, pid, policy, &p);
}

static A_UNUSED i64 getsch(i64 pid) { return syscall1(SYS_sched_getscheduler, pid); }

static A_UNUSED i64 getprio(i64 pid) {
    i32 p = -1;
    i64 r = syscall2(SYS_sched_getparam, pid, &p);
    return r < 0 ? r : p;
}

int test_main(void) {
    u8 *m = (u8 *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", (i64)m > 0, (i64)m);
    expect("unmap_second", syscall2(SYS_munmap, m + A_PAGE, A_PAGE), 0);
    i32 *last = (i32 *)(m + A_PAGE - 4);
    *last = 0;
    expect("page_end", syscall3(SYS_sched_setscheduler, 0, SCHED_BATCH, last), 0);
    expect("page_end_batch", getsch(0), SCHED_BATCH);
    expect("straddle", syscall3(SYS_sched_setscheduler, 0, SCHED_OTHER, m + A_PAGE - 2), -EFAULT);
    expect("addr_1", syscall3(SYS_sched_setscheduler, 0, SCHED_OTHER, 1), -EFAULT);
    expect("kernel", syscall3(SYS_sched_setscheduler, 0, SCHED_OTHER, 0xffff800000000000UL), -EFAULT);
    expect("noncanonical", syscall3(SYS_sched_setscheduler, 0, SCHED_OTHER, 0x0000800000000000UL), -EFAULT);
    expect("unmapped", syscall3(SYS_sched_setscheduler, 0, SCHED_OTHER, m + A_PAGE), -EFAULT);
    expect("protect", syscall3(SYS_mprotect, m, A_PAGE, PROT_NONE), 0);
    expect("prot_none", syscall3(SYS_sched_setscheduler, 0, SCHED_OTHER, last), -EFAULT);
    expect("still_batch", getsch(0), SCHED_BATCH);
    return finish();
}
