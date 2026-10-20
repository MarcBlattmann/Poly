/* exit in a thread ends only that thread: the process and its other
 * threads keep running; the thread's clear_child_tid word is cleared. */
#include "a_sys.h"

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static volatile i32 ctid1, ctid2;
static volatile i32 go;
static volatile i64 second_ran;

static int first(void *arg) {
    (void)arg;
    return 5; /* a_clone passes this to exit */
}

static int second(void *arg) {
    (void)arg;
    while (!go) a_relax();
    second_ran = syscall0(SYS_gettid);
    return 0;
}

int test_main(void) {
    ctid1 = ctid2 = -1;
    long t1 = a_clone(THREAD_FLAGS | CLONE_CHILD_CLEARTID, a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&ctid1, 0, first, 0);
    int ended1 = a_spin_until(&ctid1, 0, A_SPIN_LIMIT);
    long t2 = a_clone(THREAD_FLAGS | CLONE_CHILD_CLEARTID, a_stacks[1] + A_STACK_SIZE, 0, (i32 *)&ctid2, 0, second, 0);
    go = 1;
    int ended2 = a_spin_until(&ctid2, 0, A_SPIN_LIMIT);
    check("first_thread", t1 > 0, t1);
    check("first_exit_cleared_ctid", ended1, ctid1);
    check("second_thread", t2 > 0, t2);
    check("second_ran_after_first_exited", ended2 && second_ran == t2, second_ran);
    expect("process_still_running", syscall0(SYS_getpid), syscall0(SYS_gettid));
    return finish();
}
