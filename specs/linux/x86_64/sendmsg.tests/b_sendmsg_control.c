/* sendmsg control messages: SCM_RIGHTS with a descriptor that is not open
 * is EBADF; SCM_RIGHTS on a UDP socket is accepted (the datagram is
 * sent); SCM_CREDENTIALS with the
 * sender's own pid, uid and gid is accepted and arrives as sent at a
 * receiver with SO_PASSCRED; as root another existing pid is accepted, a
 * pid that does not exist is ESRCH. */
#include "poly_ipc.h"

static i64 send_cmsg(int sock, int type, const void *payload, int plen) {
    u64 cbuf[8];
    memset(cbuf, 0, sizeof cbuf);
    struct k_cmsghdr *c = (struct k_cmsghdr *)cbuf;
    c->len = sizeof(struct k_cmsghdr) + (u64)plen;
    c->level = SOL_SOCKET;
    c->type = type;
    memcpy((char *)cbuf + sizeof(struct k_cmsghdr), payload, (u64)plen);
    struct k_iovec iov = { "m", 1 };
    struct k_msghdr m;
    memset(&m, 0, sizeof m);
    m.iov = &iov;
    m.iovlen = 1;
    m.control = cbuf;
    m.controllen = (sizeof(struct k_cmsghdr) + (u64)plen + 7) & ~7UL;
    return syscall3(SYS_sendmsg, sock, &m, 0);
}

int test_main(void) {
    int sv[2];
    syscall4(SYS_socketpair, AF_UNIX, SOCK_DGRAM, 0, sv);
    int bad = 77;
    i64 r = send_cmsg(sv[0], SCM_RIGHTS, &bad, 4);
    check("rights_closed_fd_ebadf", r == -EBADF, r);
    int u = (int)syscall3(SYS_socket, AF_INET, SOCK_DGRAM, 0);
    struct k_sockaddr_in dst = in_loopback(9);
    syscall3(SYS_connect, u, &dst, sizeof dst);
    int zero = 0;
    r = send_cmsg(u, SCM_RIGHTS, &zero, 4);
    check("rights_on_udp_accepted_and_ignored", r == 1, r);

    int one = 1;
    syscall5(SYS_setsockopt, sv[1], SOL_SOCKET, SO_PASSCRED, &one, 4);
    struct k_ucred cr = { (int)getpid(), (unsigned)syscall0(SYS_getuid), (unsigned)syscall0(SYS_getgid) };
    r = send_cmsg(sv[0], SCM_CREDENTIALS, &cr, sizeof cr);
    check("own_credentials_ok", r == 1, r);
    u64 rb[8];
    memset(rb, 0, sizeof rb);
    char d;
    struct k_iovec iov = { &d, 1 };
    struct k_msghdr m;
    memset(&m, 0, sizeof m);
    m.iov = &iov;
    m.iovlen = 1;
    m.control = rb;
    m.controllen = sizeof rb;
    syscall3(SYS_recvmsg, sv[1], &m, 0);
    struct k_ucred got;
    memcpy(&got, (char *)rb + 16, sizeof got);
    check("credentials_arrive", got.pid == cr.pid && got.uid == cr.uid && got.gid == cr.gid, got.pid);
    struct k_ucred initp = { 1, 0, 0 };
    r = send_cmsg(sv[0], SCM_CREDENTIALS, &initp, sizeof initp);
    check("root_other_existing_pid_ok", r == 1, r);
    struct k_ucred none = { 4194304 + 13, 0, 0 };
    r = send_cmsg(sv[0], SCM_CREDENTIALS, &none, sizeof none);
    check("missing_pid_esrch", r == -ESRCH, r);
    return finish();
}
