/* tkill has no thread-group check: it reaches a thread of another process
 * (a child's second thread) by its tid alone. A signal whose default action
 * terminates, sent to any one thread, ends the whole process: the parent
 * sees the child killed by that signal.
 * poly-diff: syscalls=off (two processes, two threads)
 */
#include "poly_ipc.h"

static char stack[65536] __attribute__((aligned(16)));
static volatile int ctid;
static volatile unsigned *page;

static int sleeper(void *arg) {
    (void)arg;
    page[1] = (unsigned)gettid();
    gate_open(&page[0]);
    for (;;) sleep_ms(1000);
    return 0;
}

int test_main(void) {
    page = shared_page(4096);
    check("shared_page", page != 0, 0);
    i64 c = fork();
    if (c == 0) {
        i64 t = poly_clone(THREAD_FLAGS, stack + sizeof stack, (int *)&ctid, (int *)&ctid, sleeper, 0);
        if (t <= 0) syscall1(SYS_exit_group, 3);
        for (;;) sleep_ms(1000);
    }
    check("fork", c > 0, c);
    gate_wait(&page[0]);
    i64 t = page[1];
    check("child_thread_is_not_child_pid", t != c && t > 0, 0);
    i64 r = syscall2(SYS_tkill, t, 0);
    check("other_process_thread_signal_zero", r == 0, r);
    r = syscall2(SYS_tkill, t, 65);
    check("other_process_thread_bad_signal", r == -EINVAL, r);
    r = syscall2(SYS_tkill, t, SIGTERM);
    check("other_process_thread_sigterm", r == 0, r);
    int st = wait_status(c);
    check("child_killed", st != -1 && W_SIGNALED(st), st);
    check("child_killed_by_sigterm", W_TERMSIG(st) == SIGTERM, W_TERMSIG(st));
    r = syscall2(SYS_tkill, t, 0);
    check("dead_thread_esrch", r == -ESRCH, r);
    r = syscall2(SYS_tkill, c, 0);
    check("dead_child_esrch", r == -ESRCH, r);
    return finish();
}
