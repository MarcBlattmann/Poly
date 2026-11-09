/* Who may do what in a network namespace. A process with uid 65534 and no
 * capabilities in the initial user namespace: raw sockets
 * (socket(AF_INET, SOCK_RAW, 255), socket(AF_PACKET, SOCK_RAW, 0x300)) are
 * EPERM, datagram sockets and rtnetlink sockets work, SIOCSIFFLAGS on lo
 * is EPERM. After unshare(CLONE_NEWUSER) (mapped to root) the network
 * namespace is still owned by the initial user namespace: raw sockets and
 * SIOCSIFFLAGS stay EPERM. After unshare(CLONE_NEWUSER | CLONE_NEWNET)
 * the new network namespace is owned by the new user namespace: raw
 * sockets work, lo can be brought up (flags 0x49), and a second unshare of
 * CLONE_NEWNET gives yet another namespace with the same single device. A
 * new network namespace's /proc/self/net/dev lists lo only, and
 * /proc/net is a link to "self/net".
 * poly-diff: needs=root (the test starts as root and becomes uid 65534; creating user namespaces is a system setting)
 * poly-diff: syscalls=off (several processes)
 */
#include "p_ns.h"

#define SIOCSIFFLAGS 0x8914
#define SIOCGIFFLAGS 0x8913
#define ETH_P_ALL_BE 0x0003 /* htons(ETH_P_ALL) = 0x0300 */

static char text[2048];

static i64 set_lo_up(void) {
    struct {
        char name[16];
        short flags;
        char pad[22];
    } r;
    memset(&r, 0, sizeof r);
    str_cpy(r.name, "lo");
    r.flags = 0x49;
    i64 s = syscall3(SYS_socket, AF_INET, SOCK_DGRAM, 0);
    i64 rc = syscall3(SYS_ioctl, s, SIOCSIFFLAGS, &r);
    if (rc == 0) {
        r.flags = 0;
        syscall3(SYS_ioctl, s, SIOCGIFFLAGS, &r);
        rc = r.flags;
    }
    a_close(s);
    return rc;
}

static i64 raw_inet(void) { return syscall3(SYS_socket, AF_INET, SOCK_RAW, 255); }
static i64 raw_packet(void) { return syscall3(SYS_socket, AF_PACKET, SOCK_RAW, 0x0300); }

static int map_self(void) {
    char buf[64];
    p_map_text(buf, 0, 65534, 1);
    if (p_write_file("/proc/self/setgroups", "deny") < 0) return -1;
    if (p_write_file("/proc/self/uid_map", buf) < 0) return -2;
    if (p_write_file("/proc/self/gid_map", buf) < 0) return -3;
    return 0;
}

static int nobody_plain(void) {
    expect("drop", p_drop_to_nobody(), 0);
    syscall2(SYS_prctl, PR_SET_DUMPABLE, 1);
    expect("raw_inet_eperm", raw_inet(), -EPERM);
    expect("raw_packet_eperm", raw_packet(), -EPERM);
    i64 d = syscall3(SYS_socket, AF_INET, SOCK_DGRAM, 0);
    check("dgram_ok", d >= 0, d);
    i64 n = syscall3(SYS_socket, AF_NETLINK, SOCK_RAW, 0);
    check("netlink_ok", n >= 0, n);
    expect("set_lo_up_eperm", set_lo_up(), -EPERM);
    return failed ? 1 : 0;
}

static int nobody_userns_only(void) {
    p_drop_to_nobody();
    syscall2(SYS_prctl, PR_SET_DUMPABLE, 1);
    expect("unshare_newuser", syscall1(SYS_unshare, CLONE_NEWUSER), 0);
    expect("map_self", map_self(), 0);
    expect("raw_inet_still_eperm", raw_inet(), -EPERM);
    expect("raw_packet_still_eperm", raw_packet(), -EPERM);
    expect("set_lo_up_still_eperm", set_lo_up(), -EPERM);
    return failed ? 1 : 0;
}

static int nobody_userns_and_net(void) {
    p_drop_to_nobody();
    syscall2(SYS_prctl, PR_SET_DUMPABLE, 1);
    expect("unshare_newuser_newnet", syscall1(SYS_unshare, CLONE_NEWUSER | CLONE_NEWNET), 0);
    expect("map_self", map_self(), 0);
    i64 r = raw_inet();
    check("raw_inet_ok", r >= 0, r);
    i64 p = raw_packet();
    check("raw_packet_ok", p >= 0, p);
    expect("set_lo_up_flags", set_lo_up(), 0x49);

    i64 first = p_ns_ino("/proc/self/ns/net");
    expect("unshare_newnet_again", syscall1(SYS_unshare, CLONE_NEWNET), 0);
    check("another_netns", p_ns_ino("/proc/self/ns/net") != first, 0);
    p_read_file("/proc/self/net/dev", text, sizeof text);
    int lines = 0;
    for (const char *c = text; *c; c++)
        if (*c == '\n') lines++;
    expect("dev_has_three_lines", lines, 3);
    check("dev_lists_lo", p_find(text, "    lo:") != P_NULL, 0);
    expect("second_lo_is_down", set_lo_up(), 0x49); /* lo of the fresh namespace could be set up again */
    return failed ? 1 : 0;
}

static int root_body(void) {
    char link[32];
    expect("proc_net_link", p_readlink("/proc/net", link, sizeof link) > 0, 1);
    check("proc_net_is_self_net", str_eq(link, "self/net"), 0);
    i64 a = p_ns_ino("/proc/self/ns/net");
    expect("unshare_newnet", syscall1(SYS_unshare, CLONE_NEWNET), 0);
    i64 b = p_ns_ino("/proc/self/ns/net");
    expect("unshare_newnet_2", syscall1(SYS_unshare, CLONE_NEWNET), 0);
    i64 c = p_ns_ino("/proc/self/ns/net");
    check("three_different_namespaces", a != b && b != c && a != c, 0);
    p_read_file("/proc/self/net/dev", text, sizeof text);
    int lines = 0;
    for (const char *p = text; *p; p++)
        if (*p == '\n') lines++;
    expect("dev_three_lines", lines, 3);
    return failed ? 1 : 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    p_run("root_exit_0", 0, root_body);
    p_run("nobody_exit_0", 0, nobody_plain);
    p_run("nobody_userns_exit_0", 0, nobody_userns_only);
    p_run("nobody_userns_net_exit_0", 0, nobody_userns_and_net);
    return finish();
}
