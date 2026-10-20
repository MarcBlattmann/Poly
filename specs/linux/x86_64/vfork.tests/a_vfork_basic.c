/* vfork: the parent is suspended until the child exits; the child runs in
 * the parent's memory, so what it stores is seen by the parent. */
#include "a_sys.h"

static volatile i64 shared_word;

int test_main(void) {
    a_block_sigchld();
    i64 me = syscall0(SYS_getpid);
    shared_word = 1;
    i64 pid = syscall0(SYS_vfork);
    if (pid == 0) {
        shared_word = 2;
        syscall1(SYS_exit_group, 3);
    }
    /* The child has ended by now: its store is visible. */
    i64 seen = shared_word;
    check("vfork", pid > 0, pid);
    check("child_pid_differs", pid != me, pid);
    expect("child_wrote_parent_memory", seen, 2);
    i32 status = 0;
    expect("wait4", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    check("child_exit_3", A_WIFEXITED(status) && A_WEXITSTATUS(status) == 3, status);
    expect("no_more_children", syscall4(SYS_wait4, -1, &status, WNOHANG, 0), -ECHILD);
    return finish();
}
