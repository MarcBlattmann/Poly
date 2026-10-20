/* Blocking epoll_wait across processes: a timeout of -1 waits until a
 * child writes to a watched pipe; a handled signal ends the wait with
 * EINTR (never restarted); a child's exit closing the last write end
 * reports EPOLLHUP.
 * poly-diff: syscalls=off (another process and a timer signal)
 */
#include "poly_ipc.h"

#define ITIMER_REAL 0
struct k_itimerval {
    i64 interval_sec, interval_usec, value_sec, value_usec;
};
static void on_alarm(int sig) { (void)sig; }

int test_main(void) {
    int e = (int)syscall1(SYS_epoll_create1, 0);
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    struct k_epoll_event ev = { EPOLLIN, 77 }, out[2];
    syscall4(SYS_epoll_ctl, e, EPOLL_CTL_ADD, p[0], &ev);
    i64 pid = fork();
    if (pid == 0) {
        sleep_ms(30);
        write(p[1], "x", 1);
        sleep_ms(30);
        exit_group(0);
    }
    close(p[1]);
    i64 r = syscall4(SYS_epoll_wait, e, out, 2, -1);
    check("woken_by_child_write", r == 1 && out[0].data == 77 && out[0].events == EPOLLIN, r);
    char c;
    read(p[0], &c, 1);
    r = syscall4(SYS_epoll_wait, e, out, 2, 5000);
    check("hup_when_writer_exits", r == 1 && out[0].events == EPOLLHUP, out[0].events);
    wait_exit(pid);
    syscall4(SYS_epoll_ctl, e, EPOLL_CTL_DEL, p[0], 0);

    sig_handler(SIGALRM, on_alarm, SA_RESTART);
    struct k_itimerval it = { 0, 0, 0, 20000 };
    syscall3(SYS_setitimer, ITIMER_REAL, &it, 0);
    r = syscall4(SYS_epoll_wait, e, out, 2, -1);
    check("signal_eintr", r == -EINTR, r);
    return finish();
}
