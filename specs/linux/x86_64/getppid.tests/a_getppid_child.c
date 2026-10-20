/* getppid: a child's parent is the process that created it, whether
 * made by fork, vfork or clone. */
#include "a_sys.h"

static volatile i32 done;
static volatile i64 clone_ppid;

static int child(void *arg) {
    (void)arg;
    clone_ppid = syscall0(SYS_getppid);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    i64 me = syscall0(SYS_getpid);
    i32 st;
    i64 pid = a_fork_sync();
    if (pid == 0) {
        sh[0] = (i32)syscall0(SYS_getppid);
        syscall1(SYS_exit_group, 0);
    }
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("fork_child_parent", sh[0], me);
    static volatile i64 vppid;
    pid = syscall0(SYS_vfork);
    if (pid == 0) {
        vppid = syscall0(SYS_getppid);
        syscall1(SYS_exit_group, 0);
    }
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("vfork_child_parent", vppid, me);
    pid = a_spawn(0, &done, child, 0);
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("clone_child_parent", clone_ppid, me);
    return finish();
}
