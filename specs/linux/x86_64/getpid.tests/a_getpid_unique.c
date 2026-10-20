/* getpid: four children alive at once all have different pids, none
 * equal to the parent's; vfork children too. */
#include "a_sys.h"

static volatile i32 go[4], done[4];
static volatile i64 pids[4];

static int child(void *arg) {
    int i = (int)(i64)arg;
    pids[i] = syscall0(SYS_getpid);
    while (!go[i]) a_relax();
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    i64 me = syscall0(SYS_getpid);
    long ret[4];
    for (int i = 0; i < 4; i++) {
        ret[i] = a_spawn(i, &done[i], child, (void *)(i64)i);
        /* Let child i make its system call before the next one starts. */
        while (pids[i] == 0) a_relax();
    }
    int distinct = 1, matches = 1;
    for (int i = 0; i < 4; i++) {
        matches &= pids[i] == ret[i];
        distinct &= pids[i] != me;
        for (int j = 0; j < i; j++) distinct &= pids[i] != pids[j];
    }
    /* Let them exit one at a time, so the order is always the same. */
    for (int i = 0; i < 4; i++) {
        go[i] = 1;
        a_spin_until(&done[i], 0, A_SPIN_LIMIT);
    }
    i32 st;
    for (int i = 0; i < 4; i++) syscall4(SYS_wait4, ret[i], &st, 0, 0);
    check("each_child_got_returned_pid", matches, 0);
    check("all_distinct", distinct, 0);
    static volatile i64 vpid;
    i64 v = syscall0(SYS_vfork);
    if (v == 0) {
        vpid = syscall0(SYS_getpid);
        syscall1(SYS_exit_group, 0);
    }
    syscall4(SYS_wait4, v, &st, 0, 0);
    check("vfork_child_own_pid", vpid == v && v != me, vpid);
    return finish();
}
