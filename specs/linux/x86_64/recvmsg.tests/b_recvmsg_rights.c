/* recvmsg with SCM_RIGHTS: descriptors sent with sendmsg arrive as new
 * descriptors (the lowest free ones) for the same open files (a pipe end
 * received still carries the data); with MSG_CMSG_CLOEXEC they get
 * FD_CLOEXEC; a control buffer too small for the descriptors sets
 * MSG_CTRUNC and the extra descriptors are closed (not installed); the
 * sender may close its copies right after sendmsg. */
#include "poly_ipc.h"

static i64 send_fds(int sock, const int *fds, int n) {
    u64 cbuf[8];
    memset(cbuf, 0, sizeof cbuf);
    struct k_cmsghdr *c = (struct k_cmsghdr *)cbuf;
    c->len = sizeof(struct k_cmsghdr) + 4 * (u64)n;
    c->level = SOL_SOCKET;
    c->type = SCM_RIGHTS;
    memcpy((char *)cbuf + sizeof(struct k_cmsghdr), fds, 4 * (u64)n);
    struct k_iovec iov = { "F", 1 };
    struct k_msghdr m;
    memset(&m, 0, sizeof m);
    m.iov = &iov;
    m.iovlen = 1;
    m.control = cbuf;
    m.controllen = (sizeof(struct k_cmsghdr) + 4 * (u64)n + 7) & ~7UL;
    return syscall3(SYS_sendmsg, sock, &m, 0);
}

int test_main(void) {
    int sv[2];
    syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, sv);
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    write(p[1], "through", 7);
    i64 r = send_fds(sv[0], p, 2);
    check("sendmsg_with_rights", r == 1, r);
    close(p[0]);
    close(p[1]);

    u64 cbuf[8];
    memset(cbuf, 0, sizeof cbuf);
    char data[4];
    struct k_iovec iov = { data, sizeof data };
    struct k_msghdr m;
    memset(&m, 0, sizeof m);
    m.iov = &iov;
    m.iovlen = 1;
    m.control = cbuf;
    m.controllen = sizeof cbuf;
    r = syscall3(SYS_recvmsg, sv[1], &m, 0);
    check("recvmsg_data_byte", r == 1 && data[0] == 'F', r);
    struct k_cmsghdr *c = (struct k_cmsghdr *)cbuf;
    check("cmsg_rights_header", c->level == SOL_SOCKET && c->type == SCM_RIGHTS && c->len == 16 + 8, (i64)c->len);
    check("controllen_set", m.controllen == 24, (i64)m.controllen);
    check("no_flags", m.flags == 0, m.flags);
    int got[2];
    memcpy(got, (char *)cbuf + 16, 8);
    check("lowest_free_fds_installed", got[0] == p[0] && got[1] == p[1], got[0] * 10 + got[1]);
    char buf[8];
    r = read(got[0], buf, sizeof buf);
    check("received_pipe_carries_data", r == 7 && mem_eq(buf, "through", 7), r);
    check("received_fd_no_cloexec", syscall2(SYS_fcntl, got[0], F_GETFD) == 0, 0);
    write(got[1], "!", 1);
    r = read(got[0], buf, 1);
    check("both_ends_same_pipe", r == 1 && buf[0] == '!', r);

    /* MSG_CMSG_CLOEXEC. */
    send_fds(sv[0], got, 1);
    memset(cbuf, 0, sizeof cbuf);
    m.control = cbuf;
    m.controllen = sizeof cbuf;
    syscall3(SYS_recvmsg, sv[1], &m, MSG_CMSG_CLOEXEC);
    int third;
    memcpy(&third, (char *)cbuf + 16, 4);
    check("cmsg_cloexec_sets_flag", syscall2(SYS_fcntl, third, F_GETFD) == FD_CLOEXEC, third);
    close(third);

    /* Too small a control buffer: MSG_CTRUNC, nothing installed. */
    send_fds(sv[0], got, 2);
    memset(cbuf, 0, sizeof cbuf);
    m.control = cbuf;
    m.controllen = 20; /* header plus one descriptor */
    r = syscall3(SYS_recvmsg, sv[1], &m, 0);
    check("ctrunc_data_still_read", r == 1, r);
    check("ctrunc_flag", (m.flags & MSG_CTRUNC) != 0, m.flags);
    int one;
    memcpy(&one, (char *)cbuf + 16, 4);
    check("ctrunc_one_installed", one == third, one);
    int next = (int)syscall1(SYS_dup, 0);
    check("second_not_installed", next == one + 1, next);
    return finish();
}
