/* exit from the main thread while another thread runs: the process lives
 * on until its last thread exits, and wait4 then reports the last
 * thread's exit code. The child process here shares memory with the test
 * (CLONE_VM), so the test can watch its threads end without system calls. */
#include "a_sys.h"

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static volatile i32 main_done, worker_done, worker_ran;

static int worker(void *arg) {
    (void)arg;
    /* Wait until the child's main thread has gone. */
    while (main_done != 0) a_relax();
    worker_ran = 1;
    return 66;
}

static int child_main(void *arg) {
    (void)arg;
    worker_done = -1;
    a_clone(THREAD_FLAGS | CLONE_CHILD_CLEARTID, a_stacks[1] + A_STACK_SIZE, 0, (i32 *)&worker_done, 0, worker, 0);
    return 3; /* exit(3): the main thread only */
}

int test_main(void) {
    a_block_sigchld();
    worker_done = -1;
    long pid = a_spawn(0, &main_done, child_main, 0);
    int ended = a_spin_until(&worker_done, 0, A_SPIN_LIMIT);
    i32 st = 0;
    i64 w = syscall4(SYS_wait4, pid, &st, 0, 0);
    check("spawn", pid > 0, pid);
    check("worker_ended", ended, worker_done);
    expect("wait4", w, pid);
    expect("worker_ran_after_main_exit", worker_ran, 1);
    check("status_is_last_threads_code", A_WIFEXITED(st) && A_WEXITSTATUS(st) == 66, st);
    return finish();
}
