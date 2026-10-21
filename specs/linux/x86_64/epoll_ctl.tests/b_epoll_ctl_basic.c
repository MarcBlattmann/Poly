/* epoll_ctl: ADD, MOD and DEL; adding twice is EEXIST; MOD or DEL of an
 * unwatched descriptor is ENOENT; a regular file cannot be watched
 * (EPERM); the epoll fd itself cannot be added to itself (EINVAL); bad
 * descriptors are EBADF; the target must be an epoll fd (EINVAL); an
 * unknown op is EINVAL; a bad event pointer is EFAULT (DEL ignores the
 * event); EPOLLEXCLUSIVE is refused with MOD (EINVAL). Closing a watched
 * descriptor removes it. */
#include "poly_ipc.h"

#define SYS_unlinkat 263

int test_main(void) {
    int e = (int)syscall1(SYS_epoll_create1, 0);
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    struct k_epoll_event ev = { EPOLLIN, 1 }, out[4];

    i64 r = syscall4(SYS_epoll_ctl, e, EPOLL_CTL_ADD, p[0], &ev);
    check("add_ok", r == 0, r);
    r = syscall4(SYS_epoll_ctl, e, EPOLL_CTL_ADD, p[0], &ev);
    check("add_twice_eexist", r == -EEXIST, r);
    r = syscall4(SYS_epoll_ctl, e, EPOLL_CTL_MOD, p[1], &ev);
    check("mod_unwatched_enoent", r == -ENOENT, r);
    r = syscall4(SYS_epoll_ctl, e, EPOLL_CTL_DEL, p[1], &ev);
    check("del_unwatched_enoent", r == -ENOENT, r);

    write(p[1], "x", 1);
    r = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("ready_after_add", r == 1 && out[0].data == 1, r);

    /* MOD: new events and data. */
    ev.events = EPOLLOUT;
    ev.data = 2;
    r = syscall4(SYS_epoll_ctl, e, EPOLL_CTL_MOD, p[0], &ev);
    check("mod_ok", r == 0, r);
    r = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("mod_to_pollout_read_end_not_ready", r == 0, r);
    ev.events = EPOLLIN;
    ev.data = 3;
    syscall4(SYS_epoll_ctl, e, EPOLL_CTL_MOD, p[0], &ev);
    r = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("mod_back_new_data", r == 1 && out[0].data == 3, (i64)out[0].data);

    r = syscall4(SYS_epoll_ctl, e, EPOLL_CTL_DEL, p[0], 0);
    check("del_null_event_ok", r == 0, r);
    r = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("deleted_not_reported", r == 0, r);

    /* Closing the only descriptor of a watched file removes it. */
    int q[2];
    syscall2(SYS_pipe2, q, 0);
    ev.events = EPOLLOUT;
    ev.data = 9;
    syscall4(SYS_epoll_ctl, e, EPOLL_CTL_ADD, q[1], &ev);
    r = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("watched_write_end_ready", r == 1, r);
    close(q[1]);
    r = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("closed_fd_removed", r == 0, r);
    close(q[0]);

    /* A duplicate keeps the registration alive after closing the original. */
    syscall2(SYS_pipe2, q, 0);
    syscall4(SYS_epoll_ctl, e, EPOLL_CTL_ADD, q[1], &ev);
    int d = (int)syscall1(SYS_dup, q[1]);
    close(q[1]);
    r = syscall4(SYS_epoll_wait, e, out, 4, 0);
    check("dup_keeps_registration", r == 1 && out[0].data == 9, r);
    r = syscall4(SYS_epoll_ctl, e, EPOLL_CTL_DEL, d, 0);
    check("dup_is_another_key_enoent", r == -ENOENT, r);
    close(d);
    close(q[0]);

    /* Regular files: EPERM. */
    i64 f = syscall4(SYS_openat, AT_FDCWD, "/tmp/b_epoll_ctl_file", O_CREAT | O_RDWR | O_TRUNC, 0600);
    r = syscall4(SYS_epoll_ctl, e, EPOLL_CTL_ADD, f, &ev);
    check("regular_file_eperm", r == -EPERM, r);
    close((int)f);
    syscall3(SYS_unlinkat, AT_FDCWD, "/tmp/b_epoll_ctl_file", 0);

    r = syscall4(SYS_epoll_ctl, e, EPOLL_CTL_ADD, e, &ev);
    check("self_einval", r == -EINVAL, r);
    r = syscall4(SYS_epoll_ctl, e, EPOLL_CTL_ADD, 70, &ev);
    check("bad_target_fd_ebadf", r == -EBADF, r);
    r = syscall4(SYS_epoll_ctl, 70, EPOLL_CTL_ADD, p[0], &ev);
    check("bad_epoll_fd_ebadf", r == -EBADF, r);
    r = syscall4(SYS_epoll_ctl, p[1], EPOLL_CTL_ADD, p[0], &ev);
    check("not_epoll_fd_einval", r == -EINVAL, r);
    r = syscall4(SYS_epoll_ctl, e, 4, p[0], &ev);
    check("unknown_op_einval", r == -EINVAL, r);
    r = syscall4(SYS_epoll_ctl, e, EPOLL_CTL_ADD, p[0], 8);
    check("bad_event_pointer_efault", r == -EFAULT, r);

    ev.events = EPOLLIN | EPOLLEXCLUSIVE;
    r = syscall4(SYS_epoll_ctl, e, EPOLL_CTL_ADD, p[0], &ev);
    check("exclusive_add_ok", r == 0, r);
    r = syscall4(SYS_epoll_ctl, e, EPOLL_CTL_MOD, p[0], &ev);
    check("exclusive_mod_einval", r == -EINVAL, r);
    return finish();
}
