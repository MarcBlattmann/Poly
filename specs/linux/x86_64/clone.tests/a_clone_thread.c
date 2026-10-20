/* clone with thread flags: the new thread shares the process id, memory
 * and descriptors, gets its own thread id (written to *parent_tid), and
 * its end clears *child_tid. A thread is not a child to wait for. */
#include "a_sys.h"

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static volatile i32 ptid, ctid;
static volatile i64 t_pid, t_tid, t_ppid, t_fd;

static int thread(void *arg) {
    (void)arg;
    t_pid = syscall0(SYS_getpid);
    t_tid = syscall0(SYS_gettid);
    t_ppid = syscall0(SYS_getppid);
    t_fd = a_open("/tmp", O_RDONLY | O_DIRECTORY, 0);
    return 0;
}

int test_main(void) {
    i64 pid = syscall0(SYS_getpid), ppid = syscall0(SYS_getppid);
    ctid = -1;
    long tid = a_clone(THREAD_FLAGS | CLONE_PARENT_SETTID | CLONE_CHILD_CLEARTID, a_stacks[0] + A_STACK_SIZE,
                       (i32 *)&ptid, (i32 *)&ctid, 0, thread, 0);
    int ended = a_spin_until(&ctid, 0, A_SPIN_LIMIT);
    check("clone_thread", tid > 0, tid);
    check("thread_ended_cleared_ctid", ended, ctid);
    expect("parent_tid_written", ptid, tid);
    check("thread_tid_differs", tid != pid, tid);
    expect("thread_sees_own_tid", t_tid, tid);
    expect("thread_shares_pid", t_pid, pid);
    expect("thread_shares_ppid", t_ppid, ppid);
    check("thread_opened_fd", t_fd >= 3, t_fd);
    expect("fd_shared_with_process", syscall3(SYS_fcntl, t_fd, F_GETFD, 0), 0);
    a_close(t_fd);
    i32 status;
    expect("thread_not_waitable_echild", syscall4(SYS_wait4, tid, &status, __WALL, 0), -ECHILD);
    expect("gettid_main_is_pid", syscall0(SYS_gettid), pid);
    return finish();
}
