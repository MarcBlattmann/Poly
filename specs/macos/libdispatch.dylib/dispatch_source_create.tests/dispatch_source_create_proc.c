/* A PROC source with DISPATCH_PROC_EXIT reports a child's exit; its data
 * has the exit flag. The child is a copy of this process that exits at
 * once with a known status. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"
#include <sys/wait.h>

int test_main(void) {
    int fds[2];
    if (pipe(fds) != 0) return 1;
    pid_t pid = fork();
    if (pid == 0) {
        char c;
        read(fds[0], &c, 1); /* wait until the parent has the source ready */
        _exit(7);
    }
    dispatch_queue_t q = dispatch_queue_create("com.poly.proc", DISPATCH_QUEUE_SERIAL);
    dispatch_source_t s = dispatch_source_create(DISPATCH_SOURCE_TYPE_PROC, (uintptr_t)pid, DISPATCH_PROC_EXIT, q);
    check("a proc source is made", s != NULL, 0);
    check("get_handle is the pid", dispatch_source_get_handle(s) == (uintptr_t)pid, 0);
    check("get_mask is what was asked for", dispatch_source_get_mask(s) == DISPATCH_PROC_EXIT, (long long)dispatch_source_get_mask(s));
    dispatch_semaphore_t got = dispatch_semaphore_create(0);
    __block unsigned long data = 0;
    dispatch_source_set_event_handler(s, ^{ data = dispatch_source_get_data(s); dispatch_semaphore_signal(got); });
    dispatch_resume(s);
    gcd_pause_ms(50);
    check("the child has not exited: no event", dispatch_semaphore_wait(got, DISPATCH_TIME_NOW) != 0, 0);
    write(fds[1], "x", 1);
    check("the child exited: the handler ran", gcd_wait(got), 0);
    check("the data has the exit flag", (data & DISPATCH_PROC_EXIT) != 0, (long long)data);
    int st = 0;
    waitpid(pid, &st, 0);
    check("the child's status is still collectable by waitpid (7)", WIFEXITED(st) && WEXITSTATUS(st) == 7, st);
    dispatch_source_cancel(s);
    value("DISPATCH_PROC_EXIT", DISPATCH_PROC_EXIT);
    value("DISPATCH_PROC_FORK", DISPATCH_PROC_FORK);
    value("DISPATCH_PROC_EXEC", DISPATCH_PROC_EXEC);
    value("DISPATCH_PROC_SIGNAL", DISPATCH_PROC_SIGNAL);
    return finish();
}
