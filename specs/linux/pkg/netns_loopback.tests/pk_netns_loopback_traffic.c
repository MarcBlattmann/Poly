/* Traffic in a new network namespace. With lo down: bind to 127.0.0.1
 * works, connecting to 127.0.0.1 or to 10.0.0.1 is ENETUNREACH;
 * socketpair and Unix sockets (path and abstract) work. After lo is up
 * (SIOCSIFFLAGS) TCP and UDP on 127.0.0.1 work (listen on port 0,
 * getsockname, connect, accept, an exchange of data in both directions;
 * sendto/recvfrom), connecting to 127.0.0.1 on a port nobody listens on
 * is ECONNREFUSED, to 10.0.0.1 ENETUNREACH. The namespace is private: a
 * process outside connecting to the TCP port the child listens on gets
 * ECONNREFUSED (or ENETUNREACH if its own lo were down) and to the
 * child's abstract Unix socket ECONNREFUSED, while a Unix socket bound to
 * a file system path is reachable from outside (connect returns 0).
 * poly-diff: needs=root (new network namespace)
 * poly-diff: syscalls=off (two processes; the port number varies)
 */
#include "p_ns.h"

#define SIOCSIFFLAGS 0x8914
#define SOCK_PATH "/tmp/pk_netns_traffic.sock"

struct sa_in {
    u16 family;
    u8 port[2]; /* network order */
    u8 addr[4];
    u8 zero[8];
};

struct sa_un {
    u16 family;
    char path[108];
};

static struct sa_in mk_in(u8 a, u8 b, u8 c, u8 d, u16 port) {
    struct sa_in s;
    memset(&s, 0, sizeof s);
    s.family = AF_INET;
    s.port[0] = (u8)(port >> 8);
    s.port[1] = (u8)port;
    s.addr[0] = a;
    s.addr[1] = b;
    s.addr[2] = c;
    s.addr[3] = d;
    return s;
}

static u16 port_of(struct sa_in *s) { return (u16)((s->port[0] << 8) | s->port[1]); }

static i64 sock_(int dom, int type) { return syscall3(SYS_socket, dom, type, 0); }
static i64 bind_in(i64 fd, struct sa_in *s) { return syscall3(SYS_bind, fd, s, sizeof *s); }
static i64 connect_in(i64 fd, struct sa_in *s) { return syscall3(SYS_connect, fd, s, sizeof *s); }

static size_t un_path(struct sa_un *s, const char *path) {
    memset(s, 0, sizeof *s);
    s->family = AF_UNIX;
    str_cpy(s->path, path);
    return 2 + str_len(path) + 1;
}

static size_t un_abstract(struct sa_un *s, const char *name) {
    memset(s, 0, sizeof *s);
    s->family = AF_UNIX;
    s->path[0] = 0;
    str_cpy(s->path + 1, name);
    return 2 + 1 + str_len(name);
}

static i64 sendto_(i64 fd, const void *b, u64 n, const void *addr, u64 alen) {
    return syscall6(SYS_sendto, fd, (i64)b, (i64)n, 0, (i64)addr, (i64)alen);
}

static i64 up_lo(void) {
    struct {
        char name[16];
        short flags;
        char pad[22];
    } r;
    memset(&r, 0, sizeof r);
    str_cpy(r.name, "lo");
    r.flags = 0x49;
    i64 s = sock_(AF_INET, SOCK_DGRAM);
    i64 rc = syscall3(SYS_ioctl, s, SIOCSIFFLAGS, &r);
    a_close(s);
    return rc;
}

static i32 to_parent[2], to_child[2];

static int body(void) {
    struct sa_in lo0 = mk_in(127, 0, 0, 1, 0), peer, other = mk_in(10, 0, 0, 1, 80);
    struct sa_un un;
    char buf[16];
    a_close(to_parent[0]);
    a_close(to_child[1]);

    i64 u = sock_(AF_INET, SOCK_DGRAM);
    expect("udp_bind_lo_down", bind_in(u, &lo0), 0);
    i64 u2 = sock_(AF_INET, SOCK_DGRAM);
    peer = mk_in(127, 0, 0, 1, 9);
    expect("connect_lo_down", connect_in(u2, &peer), -ENETUNREACH);
    expect("connect_other_down", connect_in(u2, &other), -ENETUNREACH);
    i32 pair[2];
    expect("socketpair", syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, pair), 0);
    expect("pair_write", a_write(pair[0], "hi", 2), 2);
    expect("pair_read", a_read(pair[1], buf, sizeof buf), 2);
    a_close(u);
    a_close(u2);

    expect("lo_up", up_lo(), 0);

    /* TCP */
    i64 l = sock_(AF_INET, SOCK_STREAM);
    expect("tcp_bind", bind_in(l, &lo0), 0);
    expect("tcp_listen", syscall2(SYS_listen, l, 4), 0);
    struct sa_in bound;
    u64 len = sizeof bound;
    expect("getsockname", syscall3(SYS_getsockname, l, &bound, &len), 0);
    expect("getsockname_len", len, 16);
    check("port_assigned", port_of(&bound) != 0, 0);
    i64 c = sock_(AF_INET, SOCK_STREAM);
    peer = mk_in(127, 0, 0, 1, port_of(&bound));
    expect("tcp_connect", connect_in(c, &peer), 0);
    i64 a = syscall3(SYS_accept, l, 0, 0);
    check("tcp_accept", a >= 0, a);
    expect("tcp_send", sendto_(c, "ping", 4, 0, 0), 4);
    expect("tcp_recv", a_read(a, buf, sizeof buf), 4);
    check("tcp_data", mem_eq(buf, "ping", 4), 0);
    expect("tcp_reply", a_write(a, "pong", 4), 4);
    expect("tcp_recv_reply", a_read(c, buf, sizeof buf), 4);
    check("tcp_reply_data", mem_eq(buf, "pong", 4), 0);
    i64 refused = sock_(AF_INET, SOCK_STREAM);
    struct sa_in nobody = mk_in(127, 0, 0, 1, 1);
    expect("connect_refused", connect_in(refused, &nobody), -ECONNREFUSED);
    i64 away = sock_(AF_INET, SOCK_STREAM);
    expect("connect_other_up", connect_in(away, &other), -ENETUNREACH);

    /* UDP */
    i64 us = sock_(AF_INET, SOCK_DGRAM);
    expect("udp_bind", bind_in(us, &lo0), 0);
    u64 ulen = sizeof bound;
    syscall3(SYS_getsockname, us, &bound, &ulen);
    i64 uc = sock_(AF_INET, SOCK_DGRAM);
    peer = mk_in(127, 0, 0, 1, port_of(&bound));
    expect("udp_sendto", sendto_(uc, "dgram", 5, &peer, sizeof peer), 5);
    expect("udp_recv", syscall6(SYS_recvfrom, us, (i64)buf, sizeof buf, 0, 0, 0), 5);
    check("udp_data", mem_eq(buf, "dgram", 5), 0);

    /* Unix sockets: abstract (per network namespace) and path (not) */
    i64 ua = sock_(AF_UNIX, SOCK_STREAM);
    size_t n = un_abstract(&un, "pk_netns_traffic");
    expect("abstract_bind", syscall3(SYS_bind, ua, &un, n), 0);
    expect("abstract_listen", syscall2(SYS_listen, ua, 2), 0);
    i64 ub = sock_(AF_UNIX, SOCK_STREAM);
    expect("abstract_connect_same_ns", syscall3(SYS_connect, ub, &un, n), 0);
    a_unlink(SOCK_PATH);
    i64 up = sock_(AF_UNIX, SOCK_STREAM);
    n = un_path(&un, SOCK_PATH);
    expect("path_bind", syscall3(SYS_bind, up, &un, n), 0);
    expect("path_listen", syscall2(SYS_listen, up, 2), 0);

    /* tell the parent the port and wait for it */
    struct sa_in tcp_addr;
    u64 tl = sizeof tcp_addr;
    syscall3(SYS_getsockname, l, &tcp_addr, &tl);
    u16 tp = port_of(&tcp_addr);
    a_write(to_parent[1], &tp, 2);
    char go;
    a_read(to_child[0], &go, 1);
    a_unlink(SOCK_PATH);
    return failed ? 1 : 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    p_pipe(to_parent);
    p_pipe(to_child);
    i64 child = p_clone(CLONE_NEWNET);
    if (child == 0) {
        passed = failed = 0;
        int rc = body();
        a_shared()[0] = passed;
        a_shared()[1] = failed;
        p_exit(rc);
    }
    a_close(to_parent[1]);
    a_close(to_child[0]);
    u16 port = 0;
    i64 got = a_read(to_parent[0], &port, 2);
    expect("child_reported_port", got, 2);
    /* the child's checks print as they run; the parent speaks after them */
    struct sa_in peer = mk_in(127, 0, 0, 1, port);
    i64 c = sock_(AF_INET, SOCK_STREAM);
    i64 r = connect_in(c, &peer);
    check("outside_tcp_connect_fails", r == -ECONNREFUSED || r == -ENETUNREACH, r);
    struct sa_un un;
    i64 s = sock_(AF_UNIX, SOCK_STREAM);
    size_t n = un_abstract(&un, "pk_netns_traffic");
    expect("outside_abstract_connect", syscall3(SYS_connect, s, &un, n), -ECONNREFUSED);
    i64 s2 = sock_(AF_UNIX, SOCK_STREAM);
    n = un_path(&un, SOCK_PATH);
    expect("outside_path_connect", syscall3(SYS_connect, s2, &un, n), 0);
    a_write(to_child[1], "g", 1);
    int st = p_wait(child);
    expect("child_exit_0", st, 0);
    passed += a_shared()[0];
    failed += a_shared()[1];
    return finish();
}
