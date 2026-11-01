/* close_range and a shared descriptor table: a task made with
 * CLONE_FILES shares the table, so its close_range(first, last, 0)
 * closes the parent's descriptors too. With CLOSE_RANGE_UNSHARE (2) it
 * first gets a private copy of the table and closes only in that copy:
 * the parent's descriptors stay open (and the task's other descriptors,
 * outside the range, are still open in its copy).
 */
#include "a_sys.h"

#define SYS_close_range 436
#define CLOSE_RANGE_UNSHARE 2

static volatile i32 done;
static volatile i64 task_result, task_fd3_open, task_fd5_open;

static int closer(void *arg) {
    u64 flags = (u64)arg;
    task_result = syscall3(SYS_close_range, 5, 6, flags);
    task_fd3_open = syscall2(SYS_fcntl, 3, F_GETFD) >= 0;
    task_fd5_open = syscall2(SYS_fcntl, 5, F_GETFD) >= 0;
    return 0;
}

static int is_open(int fd) { return syscall2(SYS_fcntl, fd, F_GETFD) >= 0; }

static void run(u64 flags) {
    done = -1;
    long pid = a_clone(CLONE_VM | CLONE_FILES | CLONE_CHILD_CLEARTID | SIGCHLD, a_stacks[0] + A_STACK_SIZE, 0,
                       (i32 *)&done, 0, closer, (void *)flags);
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    syscall4(SYS_wait4, pid, 0, 0, 0);
}

int test_main(void) {
    a_block_sigchld();
    i64 null = a_open("/dev/null", O_RDONLY, 0);
    for (int i = 4; i <= 6; i++) syscall2(SYS_dup2, null, i);

    run(CLOSE_RANGE_UNSHARE);
    expect("unshare_result", task_result, 0);
    expect("unshare_task_closed_own", task_fd5_open, 0);
    expect("unshare_task_keeps_others", task_fd3_open, 1);
    expect("parent_fd5_still_open", is_open(5), 1);
    expect("parent_fd6_still_open", is_open(6), 1);

    run(0);
    expect("shared_result", task_result, 0);
    expect("shared_task_closed", task_fd5_open, 0);
    expect("parent_fd5_closed_too", is_open(5), 0);
    expect("parent_fd6_closed_too", is_open(6), 0);
    expect("parent_fd4_open", is_open(4), 1);
    return finish();
}
