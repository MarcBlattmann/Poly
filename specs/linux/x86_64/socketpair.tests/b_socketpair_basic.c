/* socketpair: AF_UNIX stream, datagram and seqpacket pairs, connected to
 * each other both ways; stream data has no boundaries, datagrams and
 * seqpackets keep them; the peers are unnamed (getsockname and getpeername
 * report only the family, length 2); SO_PEERCRED is the creating process;
 * SOCK_NONBLOCK/SOCK_CLOEXEC apply to both. Errors: AF_INET
 * (EOPNOTSUPP), unknown family (EAFNOSUPPORT), a protocol other than 0
 * or 1 (EPROTONOSUPPORT), bad array (EFAULT). */
#include "poly_ipc.h"

int test_main(void) {
    int sv[2] = { -1, -1 };
    i64 r = syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, sv);
    check("stream_pair", r == 0 && sv[0] == 3 && sv[1] == 4, r);
    write(sv[0], "ab", 2);
    write(sv[0], "cd", 2);
    char buf[8];
    r = read(sv[1], buf, sizeof buf);
    check("stream_no_boundaries", r == 4 && mem_eq(buf, "abcd", 4), r);
    write(sv[1], "back", 4);
    r = read(sv[0], buf, sizeof buf);
    check("both_directions", r == 4, r);
    struct k_sockaddr_un a;
    int len = sizeof a;
    memset(&a, 0x55, sizeof a);
    syscall3(SYS_getsockname, sv[0], &a, &len);
    check("unnamed_sockname", len == 2 && a.sun_family == AF_UNIX, len);
    len = sizeof a;
    syscall3(SYS_getpeername, sv[0], &a, &len);
    check("unnamed_peername", len == 2, len);
    struct k_ucred cred;
    len = sizeof cred;
    r = syscall5(SYS_getsockopt, sv[0], SOL_SOCKET, SO_PEERCRED, &cred, &len);
    check("peercred_is_creator", r == 0 && cred.pid == getpid() && cred.uid == syscall0(SYS_getuid), r);
    close(sv[0]);
    r = read(sv[1], buf, sizeof buf);
    check("eof_when_peer_closed", r == 0, r);
    close(sv[1]);

    r = syscall4(SYS_socketpair, AF_UNIX, SOCK_DGRAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0, sv);
    check("dgram_pair_flags", r == 0, r);
    check("both_nonblock", syscall2(SYS_fcntl, sv[0], F_GETFL) == (O_RDWR | O_NONBLOCK) &&
                               syscall2(SYS_fcntl, sv[1], F_GETFL) == (O_RDWR | O_NONBLOCK),
          0);
    check("both_cloexec", syscall2(SYS_fcntl, sv[0], F_GETFD) == FD_CLOEXEC &&
                              syscall2(SYS_fcntl, sv[1], F_GETFD) == FD_CLOEXEC,
          0);
    write(sv[0], "ab", 2);
    write(sv[0], "cd", 2);
    r = read(sv[1], buf, sizeof buf);
    check("dgram_boundaries", r == 2, r);
    r = read(sv[1], buf, sizeof buf);
    r = read(sv[1], buf, sizeof buf);
    check("dgram_empty_eagain", r == -EAGAIN, r);
    close(sv[0]);
    r = read(sv[1], buf, sizeof buf);
    check("dgram_peer_closed_eagain_not_eof", r == -EAGAIN, r);
    r = write(sv[1], "x", 1);
    check("dgram_write_to_closed_peer_econnrefused", r == -ECONNREFUSED, r);
    close(sv[1]);

    r = syscall4(SYS_socketpair, AF_UNIX, SOCK_SEQPACKET, 0, sv);
    write(sv[0], "abc", 3);
    write(sv[0], "de", 2);
    r = read(sv[1], buf, 2);
    check("seqpacket_cut", r == 2, r);
    r = read(sv[1], buf, sizeof buf);
    check("seqpacket_next_record", r == 2 && buf[0] == 'd', r);
    close(sv[0]);
    r = read(sv[1], buf, sizeof buf);
    check("seqpacket_eof", r == 0, r);
    close(sv[1]);

    r = syscall4(SYS_socketpair, AF_INET, SOCK_STREAM, 0, sv);
    check("inet_eopnotsupp", r == -EOPNOTSUPP, r);
    r = syscall4(SYS_socketpair, 77, SOCK_STREAM, 0, sv);
    check("unknown_family_eafnosupport", r == -EAFNOSUPPORT, r);
    r = syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 2, sv);
    check("protocol_2_eprotonosupport", r == -EPROTONOSUPPORT, r);
    r = syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 1, sv);
    check("protocol_pf_unix_accepted", r == 0, r);
    close(sv[0]);
    close(sv[1]);
    r = syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, 8);
    check("bad_array_efault", r == -EFAULT, r);
    r = syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, sv);
    check("no_fds_leaked", r == 0 && sv[0] == 3 && sv[1] == 4, sv[0]);
    return finish();
}
