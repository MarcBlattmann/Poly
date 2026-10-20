/* geteuid: children (fork, vfork, clone) and threads have the same
 * effective user id as their parent. */
#include "a_sys.h"

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static volatile i32 ctid;
static volatile i64 in_thread = -1;

static int thread(void *arg) {
    (void)arg;
    in_thread = syscall0(SYS_geteuid);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    i64 mine = syscall0(SYS_geteuid);
    volatile i32 *sh = a_shared();
    sh[0] = -1;
    i64 pid = a_fork_sync();
    if (pid == 0) {
        sh[0] = (i32)syscall0(SYS_geteuid);
        syscall1(SYS_exit_group, 0);
    }
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("fork_child_same", sh[0], mine);
    static volatile i64 in_vfork = -1;
    pid = syscall0(SYS_vfork);
    if (pid == 0) {
        in_vfork = syscall0(SYS_geteuid);
        syscall1(SYS_exit_group, 0);
    }
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("vfork_child_same", in_vfork, mine);
    ctid = -1;
    a_clone(THREAD_FLAGS | CLONE_CHILD_CLEARTID, a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&ctid, 0, thread, 0);
    a_spin_until(&ctid, 0, A_SPIN_LIMIT);
    expect("thread_same", in_thread, mine);
    return finish();
}
