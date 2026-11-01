/* mlockall with CAP_IPC_LOCK (root): RLIMIT_MEMLOCK does not apply.
 * With the soft limit 0, MCL_CURRENT and MCL_FUTURE succeed, existing
 * mappings are populated, and later mmaps of any size work and are
 * populated at once. munlockall ends it.
 * poly-diff: needs=root (CAP_IPC_LOCK bypasses RLIMIT_MEMLOCK)
 */
#include "a_sys.h"

#define MCL_CURRENT 1
#define MCL_FUTURE 2

static unsigned char vec[64];
static i64 resident(char *p, int pages) {
    i64 r = syscall3(SYS_mincore, p, (u64)pages * A_PAGE, vec);
    if (r < 0) return r;
    i64 n = 0;
    for (int i = 0; i < pages; i++) n += vec[i] & 1;
    return n;
}
static char *map(int pages) {
    return (char *)syscall6(SYS_mmap, 0, (u64)pages * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1,
                            0);
}

int test_main(void) {
    struct a_rlimit rl;
    syscall2(SYS_getrlimit, RLIMIT_MEMLOCK, &rl);
    char *a = map(4);
    rl.rlim_cur = 0;
    expect("limit_0", syscall2(SYS_setrlimit, RLIMIT_MEMLOCK, &rl), 0);
    expect("current", syscall1(SYS_mlockall, MCL_CURRENT), 0);
    expect("existing_populated", resident(a, 4), 4);
    expect("current_future", syscall1(SYS_mlockall, MCL_CURRENT | MCL_FUTURE), 0);
    char *b = map(32);
    check("big_mmap_works", (i64)b > 0, (i64)b);
    expect("big_mmap_populated", resident(b, 32), 32);
    expect("munlockall", syscall0(SYS_munlockall), 0);
    char *c = map(4);
    expect("after_munlockall_not_populated", resident(c, 4), 0);
    return finish();
}
