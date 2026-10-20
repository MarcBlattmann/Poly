/* umask: a child process inherits the mask; its own change does not
 * reach the parent. */
#include "a_sys.h"

static volatile i32 done;
static volatile i64 child_saw, child_after;

static int child(void *arg) {
    (void)arg;
    child_saw = syscall1(SYS_umask, 0111);
    child_after = syscall1(SYS_umask, 0111);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    i64 initial = syscall1(SYS_umask, 0035);
    long pid = a_spawn(0, &done, child, 0);
    /* Nothing is printed until the child has ended, so the trace order is
     * the same on every run. */
    int finished = a_spin_until(&done, 0, A_SPIN_LIMIT);
    check("spawn", pid > 0, pid);
    check("child_finished", finished, done);
    i32 status;
    expect("wait_child", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_inherited_mask", child_saw, 0035);
    expect("child_own_change", child_after, 0111);
    expect("parent_mask_unchanged", syscall1(SYS_umask, initial), 0035);
    return finish();
}
