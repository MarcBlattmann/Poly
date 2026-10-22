/* Supplementary groups are inherited by a forked child; a change in the
 * child is not seen by the parent, nor a later change in the parent by
 * the child.
 * poly-diff: needs=root (setgroups needs CAP_SETGID)
 * poly-diff: syscalls=names (two processes)
 */
#include "poly_ipc.h"

static volatile unsigned *shared;

int test_main(void) {
    shared = shared_page(4096);
    unsigned set[2] = { 77, 11 };
    i64 r = syscall2(SYS_setgroups, 2, set);
    check("setgroups_ok", r == 0, r);
    i64 pid = fork();
    if (pid == 0) {
        int code = 0;
        unsigned got[4];
        if (syscall2(SYS_getgroups, 4, got) != 2 || got[0] != 11 || got[1] != 77) code = 1;
        unsigned mine[1] = { 99 };
        if (!code && syscall2(SYS_setgroups, 1, mine) != 0) code = 2;
        gate_open(&shared[0]);
        gate_wait(&shared[1]);
        if (!code && (syscall2(SYS_getgroups, 4, got) != 1 || got[0] != 99)) code = 3;
        exit_group(code);
    }
    check("fork_ok", pid > 0, pid);
    gate_wait(&shared[0]);
    unsigned got[4];
    r = syscall2(SYS_getgroups, 4, got);
    check("parent_unchanged_by_child", r == 2 && got[0] == 11 && got[1] == 77, r);
    unsigned other[3] = { 5, 6, 7 };
    syscall2(SYS_setgroups, 3, other);
    gate_open(&shared[1]);
    int code = wait_exit(pid);
    check("child_inherited_and_isolated", code == 0, code);
    r = syscall2(SYS_getgroups, 4, got);
    check("parent_has_own_change", r == 3 && got[0] == 5 && got[2] == 7, r);
    return finish();
}
