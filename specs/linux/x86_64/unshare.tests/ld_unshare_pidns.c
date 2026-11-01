/* unshare(CLONE_NEWPID) does not move the caller: its pid stays; the
 * next child it creates is the first process of a new PID namespace and
 * sees itself as pid 1 with parent pid 0, and its own children get pids
 * 2, 3, ... there. A second unshare(CLONE_NEWPID) before that is EINVAL.
 * In the caller the child has an ordinary pid. Once that pid 1 process
 * has ended, creating another process in the namespace fails with
 * ENOMEM.
 * poly-diff: needs=root (a new PID namespace needs CAP_SYS_ADMIN)
 */
#include "a_sys.h"

#define SYS_unshare 272
#define CLONE_NEWPID 0x20000000 /* linux/sched.h */

static int in_ns(void) {
    i64 self = syscall0(SYS_getpid);
    expect("unshare_newpid", syscall1(SYS_unshare, CLONE_NEWPID), 0);
    check("own_pid_unchanged", syscall0(SYS_getpid) == self, 0);
    expect("second_newpid_einval", syscall1(SYS_unshare, CLONE_NEWPID), -EINVAL);

    volatile i32 *sh = a_shared();
    i64 c = a_fork_sync();
    if (c == 0) {
        sh[2] = (i32)syscall0(SYS_getpid);
        sh[3] = (i32)syscall0(SYS_getppid);
        i64 g = a_fork_sync();
        if (g == 0) {
            sh[4] = (i32)syscall0(SYS_getpid);
            sh[5] = (i32)syscall0(SYS_getppid);
            syscall1(SYS_exit_group, 0);
        }
        sh[6] = (i32)g;
        i32 s = -1;
        syscall4(SYS_wait4, g, &s, 0, 0);
        i64 g2 = a_fork_sync();
        if (g2 == 0) syscall1(SYS_exit_group, 0);
        sh[7] = (i32)g2;
        syscall4(SYS_wait4, g2, &s, 0, 0);
        syscall1(SYS_exit_group, 0);
    }
    check("child_pid_outside_not_1", c > 1, c);
    i32 st = -1;
    syscall4(SYS_wait4, c, &st, 0, 0);
    expect("child_exit_0", st, 0);
    expect("first_child_pid_1", sh[2], 1);
    expect("first_child_ppid_0", sh[3], 0);
    expect("grandchild_pid_2", sh[4], 2);
    expect("grandchild_ppid_1", sh[5], 1);
    expect("init_sees_grandchild_2", sh[6], 2);
    expect("next_grandchild_3", sh[7], 3);
    expect("fork_after_init_died_enomem", a_fork_sync(), -ENOMEM);
    sh[0] = passed;
    sh[1] = failed;
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, in_ns());
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("ns_child_exit_0", st, 0);
    passed += sh[0];
    failed += sh[1];
    return finish();
}
