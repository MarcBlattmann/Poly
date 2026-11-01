/* mlock2 with CAP_IPC_LOCK (root): RLIMIT_MEMLOCK does not apply. With
 * the soft limit 0 or 1 page, root still locks 4 pages (populated), and
 * len 0 is 0. Without a limit to hit, a len whose rounded end wraps
 * around (-8192) is EINVAL (22), while len ~0 rounds up to 0 and
 * succeeds; flags are still checked first.
 * poly-diff: needs=root (CAP_IPC_LOCK bypasses RLIMIT_MEMLOCK)
 */
#include "a_sys.h"

#define SYS_mlock2 325
#define MLOCK_ONFAULT 0x01

static unsigned char vec[16];
static i64 resident(char *p, int pages) {
    i64 r = syscall3(SYS_mincore, p, (u64)pages * A_PAGE, vec);
    if (r < 0) return r;
    i64 n = 0;
    for (int i = 0; i < pages; i++) n += vec[i] & 1;
    return n;
}

static i64 set_soft(u64 bytes) {
    struct a_rlimit rl;
    syscall2(SYS_getrlimit, RLIMIT_MEMLOCK, &rl);
    rl.rlim_cur = bytes;
    return syscall2(SYS_setrlimit, RLIMIT_MEMLOCK, &rl);
}

int test_main(void) {
    char *m = (char *)syscall6(SYS_mmap, 0, 8 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("limit_0", set_soft(0), 0);
    expect("root_locks_with_limit_0", syscall3(SYS_mlock2, m, 4 * A_PAGE, 0), 0);
    expect("populated", resident(m, 8), 4);
    expect("len0_with_limit_0", syscall3(SYS_mlock2, m, 0, 0), 0);
    expect("onfault_with_limit_0", syscall3(SYS_mlock2, m + 4 * A_PAGE, 2 * A_PAGE, MLOCK_ONFAULT), 0);
    expect("bad_flags_still_einval", syscall3(SYS_mlock2, m, A_PAGE, 4), -EINVAL);
    expect("limit_1_page", set_soft(A_PAGE), 0);
    expect("root_locks_over_limit", syscall3(SYS_mlock2, m, 8 * A_PAGE, 0), 0);
    expect("all_populated", resident(m, 8), 8);
    expect("wrapping_len", syscall3(SYS_mlock2, m, -8192L, 0), -EINVAL);
    expect("len_all_ones_rounds_to_0", syscall3(SYS_mlock2, m, ~0UL, 0), 0);
    expect("munlock", syscall2(SYS_munlock, m, 8 * A_PAGE), 0);
    return finish();
}
