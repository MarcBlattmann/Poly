/* getpid: a child process has its own pid, the one fork returned; a
 * thread has the process's pid. */
#include "a_sys.h"

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static volatile i32 ctid;
static volatile i64 thread_pid;

static int thread(void *arg) {
    (void)arg;
    thread_pid = syscall0(SYS_getpid);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    i64 me = syscall0(SYS_getpid);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        sh[0] = (i32)syscall0(SYS_getpid);
        syscall1(SYS_exit_group, 0);
    }
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_pid_is_fork_result", sh[0], pid);
    check("child_pid_differs", pid != me, pid);
    ctid = -1;
    a_clone(THREAD_FLAGS | CLONE_CHILD_CLEARTID, a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&ctid, 0, thread, 0);
    a_spin_until(&ctid, 0, A_SPIN_LIMIT);
    expect("thread_has_process_pid", thread_pid, me);
    expect("own_pid_unchanged", syscall0(SYS_getpid), me);
    return finish();
}
