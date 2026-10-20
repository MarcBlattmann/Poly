/* gettid: each thread has its own id, different from the process id and
 * from the other threads', and equal to what clone returned for it. */
#include "a_sys.h"

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static volatile i32 ctid[3];
static volatile i64 seen[3];

static int thread(void *arg) {
    int i = (int)(i64)arg;
    seen[i] = syscall0(SYS_gettid);
    return 0;
}

int test_main(void) {
    i64 pid = syscall0(SYS_getpid);
    long ret[3];
    for (int i = 0; i < 3; i++) {
        ctid[i] = -1;
        ret[i] = a_clone(THREAD_FLAGS | CLONE_CHILD_CLEARTID, a_stacks[i] + A_STACK_SIZE, 0, (i32 *)&ctid[i], 0,
                         thread, (void *)(i64)i);
        a_spin_until(&ctid[i], 0, A_SPIN_LIMIT);
    }
    int ok = 1;
    for (int i = 0; i < 3; i++) {
        ok &= seen[i] == ret[i] && seen[i] != pid;
        for (int j = 0; j < i; j++) ok &= seen[i] != seen[j];
    }
    check("thread_ids_distinct_and_returned", ok, seen[0]);
    expect("main_still_pid", syscall0(SYS_gettid), pid);
    return finish();
}
