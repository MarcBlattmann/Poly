/* sched_yield lets other runnable threads run: a thread that waits by
 * yielding sees a flag another thread sets; the program never hangs. */
#include "a_sys.h"

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static volatile i32 ctid, flag, may_exit;

static int setter(void *arg) {
    (void)arg;
    flag = 1;
    while (!may_exit) a_relax();
    return 0;
}

int test_main(void) {
    ctid = -1;
    a_clone(THREAD_FLAGS | CLONE_CHILD_CLEARTID, a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&ctid, 0, setter, 0);
    /* Wait without system calls first, so the number of yields below is
     * fixed: then yield exactly ten times. */
    int seen = a_spin_until(&flag, 1, A_SPIN_LIMIT);
    int ok = 1;
    for (int i = 0; i < 10; i++) ok &= syscall0(SYS_sched_yield) == 0;
    may_exit = 1;
    a_spin_until(&ctid, 0, A_SPIN_LIMIT);
    check("flag_seen", seen, flag);
    check("ten_yields_ok", ok, 0);
    return finish();
}
