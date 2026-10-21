/* socket: creating AF_UNIX (stream, datagram, seqpacket) and AF_INET
 * (stream/TCP, datagram/UDP) sockets; SOCK_NONBLOCK and SOCK_CLOEXEC in
 * the type; SO_TYPE, SO_DOMAIN and SO_PROTOCOL read back what was made
 * (protocol 0 picks TCP or UDP for AF_INET; AF_UNIX takes 0 or 1).
 * Errors: unknown family (EAFNOSUPPORT), an AF_UNIX type it lacks
 * (ESOCKTNOSUPPORT), a protocol that does not fit the type
 * (EPROTONOSUPPORT), other flag bits in the type (EINVAL). A socket
 * is O_RDWR, lseek is ESPIPE. */
#include "poly_ipc.h"

static int opt(int fd, int name) {
    int v = -1;
    int len = 4;
    syscall5(SYS_getsockopt, fd, SOL_SOCKET, name, &v, &len);
    return v;
}

int test_main(void) {
    i64 s = syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    check("unix_stream", s == 3, s);
    check("unix_stream_type", opt((int)s, SO_TYPE) == SOCK_STREAM, opt((int)s, SO_TYPE));
    check("unix_stream_domain", opt((int)s, SO_DOMAIN) == AF_UNIX, opt((int)s, SO_DOMAIN));
    check("unix_stream_protocol", opt((int)s, SO_PROTOCOL) == 0, opt((int)s, SO_PROTOCOL));
    check("rdwr", syscall2(SYS_fcntl, s, F_GETFL) == O_RDWR, syscall2(SYS_fcntl, s, F_GETFL));
    check("no_cloexec", syscall2(SYS_fcntl, s, F_GETFD) == 0, 0);
    check("lseek_espipe", syscall3(SYS_lseek, s, 0, 0) == -ESPIPE, 0);
    close((int)s);

    s = syscall3(SYS_socket, AF_UNIX, SOCK_DGRAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);
    check("unix_dgram_flags", s == 3, s);
    check("nonblock_set", syscall2(SYS_fcntl, s, F_GETFL) == (O_RDWR | O_NONBLOCK), 0);
    check("cloexec_set", syscall2(SYS_fcntl, s, F_GETFD) == FD_CLOEXEC, 0);
    check("flags_not_in_so_type", opt((int)s, SO_TYPE) == SOCK_DGRAM, opt((int)s, SO_TYPE));
    close((int)s);
    s = syscall3(SYS_socket, AF_UNIX, SOCK_SEQPACKET, 0);
    check("unix_seqpacket", s >= 0 && opt((int)s, SO_TYPE) == SOCK_SEQPACKET, s);
    close((int)s);

    s = syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    check("inet_stream", s >= 0, s);
    check("inet_stream_protocol_tcp", opt((int)s, SO_PROTOCOL) == IPPROTO_TCP, opt((int)s, SO_PROTOCOL));
    check("inet_domain", opt((int)s, SO_DOMAIN) == AF_INET, 0);
    close((int)s);
    s = syscall3(SYS_socket, AF_INET, SOCK_DGRAM, IPPROTO_UDP);
    check("inet_dgram_udp", s >= 0 && opt((int)s, SO_PROTOCOL) == IPPROTO_UDP, s);
    close((int)s);

    i64 r = syscall3(SYS_socket, 77, SOCK_STREAM, 0);
    check("unknown_family_eafnosupport", r == -EAFNOSUPPORT, r);
    r = syscall3(SYS_socket, AF_UNIX, 9, 0);
    check("unix_unknown_type_esocktnosupport", r == -ESOCKTNOSUPPORT, r);
    r = syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 1);
    check("unix_protocol_pf_unix_accepted", r >= 0, r);
    close((int)r);
    r = syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 5);
    check("unix_nonzero_protocol_eprotonosupport", r == -EPROTONOSUPPORT, r);
    r = syscall3(SYS_socket, AF_INET, SOCK_STREAM, IPPROTO_UDP);
    check("tcp_type_udp_protocol_eprotonosupport", r == -EPROTONOSUPPORT, r);
    r = syscall3(SYS_socket, AF_INET, SOCK_DGRAM, IPPROTO_TCP);
    check("udp_type_tcp_protocol_eprotonosupport", r == -EPROTONOSUPPORT, r);
    r = syscall3(SYS_socket, AF_UNIX, SOCK_STREAM | 0x100, 0);
    check("other_type_flag_einval", r == -EINVAL, r);
    r = syscall3(SYS_socket, -1, SOCK_STREAM, 0);
    check("negative_family_eafnosupport", r == -EAFNOSUPPORT, r);
    return finish();
}
