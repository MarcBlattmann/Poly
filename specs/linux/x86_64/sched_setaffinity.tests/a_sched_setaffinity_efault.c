/* A mask that cannot be read is EFAULT (14): NULL, address 1, a kernel
 * address, a non-canonical address, an unmapped page, a PROT_NONE page,
 * and 8 bytes that run into an unmapped page (while 4 bytes there with
 * len 4 work). The mask is read before the pid is looked up: a missing or
 * negative pid with an unreadable mask is EFAULT, not ESRCH. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setaffinity 203
#define SYS_sched_getaffinity 204

static u64 mask_of(i64 pid) {
    u64 m[16];
    memset(m, 0, sizeof m);
    i64 r = syscall3(SYS_sched_getaffinity, pid, sizeof m, m);
    return r == 8 ? m[0] : 0;
}

int test_main(void) {
    u64 start = mask_of(0);
    u64 lo = start & -start;
    u64 v = lo;
    expect("set_lowest", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);

    expect("null", syscall3(SYS_sched_setaffinity, 0, 8, 0), -EFAULT);
    expect("address_1", syscall3(SYS_sched_setaffinity, 0, 8, 1), -EFAULT);
    expect("kernel_address", syscall3(SYS_sched_setaffinity, 0, 8, 0xffff800000000000UL), -EFAULT);
    expect("non_canonical", syscall3(SYS_sched_setaffinity, 0, 8, 0x0000800000000000UL), -EFAULT);

    char *pg = (char *)syscall6(SYS_mmap, 0, 3 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("unmap_middle", syscall2(SYS_munmap, pg + A_PAGE, A_PAGE), 0);
    expect("protect_last", syscall3(SYS_mprotect, pg + 2 * A_PAGE, A_PAGE, PROT_NONE), 0);
    expect("unmapped_page", syscall3(SYS_sched_setaffinity, 0, 8, pg + A_PAGE), -EFAULT);
    expect("prot_none_page", syscall3(SYS_sched_setaffinity, 0, 8, pg + 2 * A_PAGE), -EFAULT);
    u32 *edge = (u32 *)(pg + A_PAGE - 4);
    *edge = (u32)~0U;
    expect("straddle_len8", syscall3(SYS_sched_setaffinity, 0, 8, edge), -EFAULT);
    expect("straddle_len5", syscall3(SYS_sched_setaffinity, 0, 5, edge), -EFAULT);
    check("failures_kept_mask", mask_of(0) == lo, 0);
    expect("edge_len4", syscall3(SYS_sched_setaffinity, 0, 4, edge), 0);
    check("edge_len4_seen", mask_of(0) == (start & 0xffffffffUL), 0);

    expect("missing_pid_null", syscall3(SYS_sched_setaffinity, 0x3fffffff, 8, 0), -EFAULT);
    expect("negative_pid_null", syscall3(SYS_sched_setaffinity, -1, 8, 0), -EFAULT);
    expect("missing_pid_unmapped", syscall3(SYS_sched_setaffinity, 0x7fffffff, 8, pg + A_PAGE), -EFAULT);
    v = start;
    expect("restore", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    return finish();
}
