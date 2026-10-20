/* getppid: all threads of a process report the same parent; a child
 * created by a thread has the thread's process as its parent. */
#include "a_sys.h"

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static volatile i32 ctid, cdone;
static volatile i64 t_ppid, grandchild_ppid, child_pid;

static int grandchild(void *arg) {
    (void)arg;
    grandchild_ppid = syscall0(SYS_getppid);
    return 0;
}

static int thread(void *arg) {
    (void)arg;
    t_ppid = syscall0(SYS_getppid);
    child_pid = a_spawn(1, &cdone, grandchild, 0);
    a_spin_until(&cdone, 0, A_SPIN_LIMIT);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    i64 me = syscall0(SYS_getpid), parent = syscall0(SYS_getppid);
    ctid = -1;
    a_clone(THREAD_FLAGS | CLONE_CHILD_CLEARTID, a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&ctid, 0, thread, 0);
    a_spin_until(&ctid, 0, A_SPIN_LIMIT);
    expect("thread_same_parent", t_ppid, parent);
    expect("child_of_thread_has_process_as_parent", grandchild_ppid, me);
    i32 st;
    expect("process_reaps_child_of_thread", syscall4(SYS_wait4, child_pid, &st, 0, 0), child_pid);
    return finish();
}
