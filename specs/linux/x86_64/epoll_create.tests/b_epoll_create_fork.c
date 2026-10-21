/* An epoll instance is shared across fork: the child inherits the
 * descriptor with its interest list, sees the same ready events, and a
 * registration the child adds is visible to the parent.
 * poly-diff: syscalls=off (a child process)
 */
#include "poly_ipc.h"

int test_main(void) {
    int e = (int)syscall1(SYS_epoll_create, 4);
    int p[2], q[2];
    syscall2(SYS_pipe2, p, 0);
    syscall2(SYS_pipe2, q, 0);
    struct k_epoll_event ev = { EPOLLOUT, 1 }, out[4];
    syscall4(SYS_epoll_ctl, e, EPOLL_CTL_ADD, p[1], &ev);
    i64 pid = fork();
    if (pid == 0) {
        int code = 0;
        if (syscall4(SYS_epoll_wait, e, out, 4, 0) != 1 || out[0].data != 1) code |= 1;
        struct k_epoll_event w = { EPOLLOUT, 2 };
        if (syscall4(SYS_epoll_ctl, e, EPOLL_CTL_ADD, q[1], &w) != 0) code |= 2;
        exit_group(code);
    }
    int code = wait_exit(pid);
    check("child_sees_inherited_registration", code >= 0 && !(code & 1), code);
    check("child_adds_registration", code >= 0 && !(code & 2), code);
    i64 r = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("parent_sees_childs_registration", r == 2, r);
    return finish();
}
