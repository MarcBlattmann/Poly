/* Calls that set the gids to values the caller already has (setgid to
 * the real gid, setresgid with -1 everywhere, setregid to the current
 * pair) succeed for any user and leave the supplementary groups exactly
 * as they were. Neither the real nor the effective gid is added to or
 * removed from the list by them.
 */
#include "poly_ipc.h"

static unsigned before[1024], after[1024];

static int same(i64 n) {
    for (int i = 0; i < 1024; i++) after[i] = 0xcccccccc;
    i64 r = syscall2(SYS_getgroups, 1024, after);
    return n > 1024 || (r == n && mem_eq(before, after, (u64)n * 4) && after[n] == 0xcccccccc);
}

int test_main(void) {
    i64 n = syscall2(SYS_getgroups, 0, 0);
    syscall2(SYS_getgroups, 1024, before);
    i64 gid = syscall0(SYS_getgid), egid = syscall0(SYS_getegid);
    i64 r = syscall3(SYS_setresgid, -1, -1, -1);
    check("setresgid_noop_ok", r == 0, r);
    check("list_same_after_setresgid", same(n), 0);
    r = syscall2(SYS_setregid, gid, egid);
    check("setregid_same_ok", r == 0, r);
    check("list_same_after_setregid", same(n), 0);
    r = syscall1(SYS_setgid, gid);
    check("setgid_real_ok", r == 0, r);
    check("list_same_after_setgid", same(n), 0);
    check("count_same", syscall2(SYS_getgroups, 0, 0) == n, 0);
    return finish();
}
