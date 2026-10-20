/* sched_yield in a child process and in the parent while the child runs:
 * both return 0 and the child completes. */
#include "a_sys.h"

static volatile i32 done;
static volatile i64 child_result = -1;

static int child(void *arg) {
    (void)arg;
    i64 r = 0;
    for (int i = 0; i < 5; i++) r |= syscall0(SYS_sched_yield);
    child_result = r;
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    long pid = a_spawn(0, &done, child, 0);
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    i64 mine = syscall0(SYS_sched_yield);
    i32 st;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("child_yields_ok", child_result, 0);
    expect("parent_yield_ok", mine, 0);
    return finish();
}
