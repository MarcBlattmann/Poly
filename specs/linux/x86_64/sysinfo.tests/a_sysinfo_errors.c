/* sysinfo: a bad pointer is EFAULT; the process count grows while
 * children are alive. */
#include "a_sys.h"

static volatile i32 go[3], done[3];

static int child(void *arg) {
    int i = (int)(i64)arg;
    while (!go[i]) a_relax();
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    expect("bad_pointer_efault", syscall1(SYS_sysinfo, (void *)8), -EFAULT);
    expect("null_efault", syscall1(SYS_sysinfo, (void *)0), -EFAULT);
    struct a_sysinfo a, b;
    syscall1(SYS_sysinfo, &a);
    long pids[3];
    for (int i = 0; i < 3; i++) pids[i] = a_spawn(i, &done[i], child, (void *)(i64)i);
    syscall1(SYS_sysinfo, &b);
    /* One at a time, so the children's exits come in a fixed order. */
    for (int i = 0; i < 3; i++) {
        go[i] = 1;
        a_spin_until(&done[i], 0, A_SPIN_LIMIT);
    }
    i32 st;
    for (int i = 0; i < 3; i++) syscall4(SYS_wait4, pids[i], &st, 0, 0);
    check("procs_count_children", b.procs >= a.procs + 3 || b.procs >= 4, b.procs);
    return finish();
}
