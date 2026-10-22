/* A forked child has the same supplementary groups as its parent, in the
 * same order.
 * poly-diff: syscalls=names (two processes)
 */
#include "poly_ipc.h"

static unsigned parent[1024], child[1024];

int test_main(void) {
    i64 n = syscall2(SYS_getgroups, 0, 0);
    i64 r = syscall2(SYS_getgroups, 1024, parent);
    check("parent_list", n > 1024 || r == n, r);
    i64 pid = fork();
    if (pid == 0) {
        i64 c = syscall2(SYS_getgroups, 0, 0);
        if (c != n) exit_group(1);
        if (syscall2(SYS_getgroups, 1024, child) != c && c <= 1024) exit_group(2);
        if (c <= 1024 && !mem_eq(parent, child, (u64)c * 4)) exit_group(3);
        exit_group(0);
    }
    check("fork_ok", pid > 0, pid);
    int code = wait_exit(pid);
    check("child_same_groups", code == 0, code);
    return finish();
}
