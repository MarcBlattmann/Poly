/* Configuring the loopback device of a new network namespace over rtnetlink
 * (socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE=0); this is how bubblewrap
 * and ip(8) bring lo up). RTM_GETLINK with NLM_F_REQUEST|NLM_F_DUMP
 * (0x301) answers one RTM_NEWLINK message (type 16) for index 1 with
 * IFLA_IFNAME "lo" (attribute length 7), IFLA_MTU 65536 and IFF_UP clear,
 * then NLMSG_DONE (type 3). RTM_NEWLINK with NLM_F_REQUEST|NLM_F_ACK
 * (0x5), ifi_index 1, ifi_flags = ifi_change = IFF_UP is acknowledged by
 * an NLMSG_ERROR message (type 2, length 36: the error 0 and the
 * request's header) and the device is up afterwards; an unknown index
 * (99) is acknowledged with error -ENODEV. RTM_GETADDR (22) for AF_INET
 * with the dump flags after that answers one RTM_NEWADDR (20): prefix
 * length 8, scope host (254), index 1, IFA_ADDRESS 127.0.0.1 and
 * IFA_LABEL "lo", then NLMSG_DONE. A process without CAP_NET_ADMIN can
 * dump but its RTM_NEWLINK is acknowledged with error -EPERM.
 * poly-diff: needs=root (new network namespace)
 * poly-diff: syscalls=off (several processes; descriptor numbers and ids vary)
 */
#include "p_ns.h"

struct nlh {
    u32 len;
    u16 type;
    u16 flags;
    u32 seq;
    u32 pid;
};
struct ifinfo {
    u8 family, pad;
    u16 type;
    i32 index;
    u32 flags;
    u32 change;
};
struct ifaddr {
    u8 family, prefixlen, flags, scope;
    u32 index;
};
struct sa_nl {
    u16 family;
    u16 pad;
    u32 pid;
    u32 groups;
};
struct rta {
    u16 len;
    u16 type;
};

#define NLM_F_REQUEST 1
#define NLM_F_ACK 4
#define NLM_F_DUMP 0x300
#define NLMSG_ERROR 2
#define NLMSG_DONE 3
#define RTM_NEWLINK 16
#define RTM_GETLINK 18
#define RTM_NEWADDR 20
#define RTM_GETADDR 22
#define IFLA_IFNAME 3
#define IFLA_MTU 4
#define IFA_ADDRESS 1
#define IFA_LABEL 3
#define IFF_UP 1

static u8 req[64] __attribute__((aligned(8)));
static u8 resp[8192] __attribute__((aligned(8)));
static i64 nl;

static i64 send_req(u16 type, u16 flags, u8 family, i32 index, u32 ifflags, u32 change, u32 len) {
    struct sa_nl to;
    memset(&to, 0, sizeof to);
    to.family = AF_NETLINK;
    memset(req, 0, sizeof req);
    struct nlh *h = (struct nlh *)req;
    h->len = len;
    h->type = type;
    h->flags = flags;
    h->seq = 77;
    struct ifinfo *i = (struct ifinfo *)(req + 16);
    i->family = family;
    i->index = index;
    i->flags = ifflags;
    i->change = change;
    return syscall6(SYS_sendto, nl, (i64)req, len, 0, (i64)&to, sizeof to);
}

/* Receive one datagram; returns its length. */
static i64 recv_msg(void) { return syscall6(SYS_recvfrom, nl, (i64)resp, sizeof resp, 0, 0, 0); }

static void *attr(struct nlh *h, size_t hdrlen, u16 type, u16 *len) {
    u8 *p = (u8 *)h + 16 + hdrlen;
    u8 *end = (u8 *)h + h->len;
    while (p + 4 <= end) {
        struct rta *a = (struct rta *)p;
        if (a->len < 4) break;
        if (a->type == type) {
            *len = a->len;
            return p + 4;
        }
        p += (a->len + 3U) & ~3U;
    }
    *len = 0;
    return P_NULL;
}

/* Dump links and describe the first answer; returns the number of
 * RTM_NEWLINK messages, sets *up, *mtu, *named_lo, *done. */
static int dump_links(int *up, u32 *mtu, int *named_lo, int *done, int *index, u16 *namelen) {
    int n = 0;
    *done = 0;
    send_req(RTM_GETLINK, NLM_F_REQUEST | NLM_F_DUMP, 0, 0, 0, 0, 32);
    for (int round = 0; round < 4 && !*done; round++) {
        i64 got = recv_msg();
        if (got <= 0) break;
        for (struct nlh *h = (struct nlh *)resp; (u8 *)h < resp + got; h = (struct nlh *)((u8 *)h + ((h->len + 3U) & ~3U))) {
            if (h->type == NLMSG_DONE) {
                *done = 1;
            } else if (h->type == RTM_NEWLINK) {
                struct ifinfo *i = (struct ifinfo *)((u8 *)h + 16);
                u16 l;
                char *name = attr(h, sizeof *i, IFLA_IFNAME, &l);
                u32 *m = attr(h, sizeof *i, IFLA_MTU, &l);
                if (n == 0) {
                    *up = (i->flags & IFF_UP) != 0;
                    *index = i->index;
                    u16 nl_;
                    name = attr(h, sizeof *i, IFLA_IFNAME, &nl_);
                    *namelen = nl_;
                    *named_lo = name && str_eq(name, "lo");
                    *mtu = m ? *m : 0;
                }
                n++;
            }
        }
    }
    return n;
}

static int nobody_body(void) {
    i64 s = syscall3(SYS_socket, AF_NETLINK, SOCK_RAW, 0);
    nl = s;
    struct sa_nl me;
    memset(&me, 0, sizeof me);
    me.family = AF_NETLINK;
    expect("nobody_bind", syscall3(SYS_bind, nl, &me, sizeof me), 0);
    expect("nobody_drop", p_drop_to_nobody(), 0);
    int up, named, done, index;
    u32 mtu;
    u16 nlen;
    expect("nobody_dump_one_link", dump_links(&up, &mtu, &named, &done, &index, &nlen), 1);
    expect("nobody_newlink_sent", send_req(RTM_NEWLINK, NLM_F_REQUEST | NLM_F_ACK, 0, 1, IFF_UP, IFF_UP, 32), 32);
    i64 got = recv_msg();
    struct nlh *h = (struct nlh *)resp;
    expect("nobody_ack_type", h->type, NLMSG_ERROR);
    expect("nobody_ack_error", got >= 20 ? *(i32 *)(resp + 16) : 1, -EPERM);
    return failed ? 1 : 0;
}

static int body(void) {
    nl = syscall3(SYS_socket, AF_NETLINK, SOCK_RAW | SOCK_CLOEXEC, 0);
    check("socket", nl >= 0, nl);
    struct sa_nl me;
    memset(&me, 0, sizeof me);
    me.family = AF_NETLINK;
    expect("bind", syscall3(SYS_bind, nl, &me, sizeof me), 0);
    u64 len = sizeof me;
    expect("getsockname", syscall3(SYS_getsockname, nl, &me, &len), 0);
    check("port_id_assigned", me.pid != 0, me.pid);

    int up, named, done, index;
    u32 mtu;
    u16 nlen;
    expect("dump_one_link", dump_links(&up, &mtu, &named, &done, &index, &nlen), 1);
    expect("dump_done", done, 1);
    expect("link_index", index, 1);
    expect("link_name_lo", named, 1);
    expect("link_name_attr_len", nlen, 7);
    expect("link_mtu", mtu, 65536);
    expect("link_down", up, 0);

    expect("newlink_up_sent", send_req(RTM_NEWLINK, NLM_F_REQUEST | NLM_F_ACK, 0, 1, IFF_UP, IFF_UP, 32), 32);
    i64 got = recv_msg();
    struct nlh *h = (struct nlh *)resp;
    expect("ack_length", got, 36);
    expect("ack_type", h->type, NLMSG_ERROR);
    expect("ack_msg_len", h->len, 36);
    expect("ack_seq", h->seq, 77);
    expect("ack_error_0", *(i32 *)(resp + 16), 0);
    expect("ack_echoes_request_type", ((struct nlh *)(resp + 20))->type, RTM_NEWLINK);
    expect("dump_after_up", dump_links(&up, &mtu, &named, &done, &index, &nlen), 1);
    expect("link_up", up, 1);

    expect("newlink_bad_index_sent", send_req(RTM_NEWLINK, NLM_F_REQUEST | NLM_F_ACK, 0, 99, IFF_UP, IFF_UP, 32), 32);
    got = recv_msg();
    expect("bad_index_type", ((struct nlh *)resp)->type, NLMSG_ERROR);
    expect("bad_index_error", *(i32 *)(resp + 16), -ENODEV);

    /* addresses of AF_INET */
    send_req(RTM_GETADDR, NLM_F_REQUEST | NLM_F_DUMP, AF_INET, 0, 0, 0, 24);
    int naddr = 0, adone = 0;
    for (int round = 0; round < 4 && !adone; round++) {
        got = recv_msg();
        if (got <= 0) break;
        for (struct nlh *m = (struct nlh *)resp; (u8 *)m < resp + got; m = (struct nlh *)((u8 *)m + ((m->len + 3U) & ~3U))) {
            if (m->type == NLMSG_DONE) {
                adone = 1;
            } else if (m->type == RTM_NEWADDR) {
                struct ifaddr *a = (struct ifaddr *)((u8 *)m + 16);
                u16 l;
                u8 *ip = attr(m, sizeof *a, IFA_ADDRESS, &l);
                char *label = attr(m, sizeof *a, IFA_LABEL, &l);
                naddr++;
                expect("addr_family", a->family, AF_INET);
                expect("addr_prefixlen", a->prefixlen, 8);
                expect("addr_scope_host", a->scope, 254);
                expect("addr_index", a->index, 1);
                check("addr_127_0_0_1", ip && ip[0] == 127 && ip[1] == 0 && ip[2] == 0 && ip[3] == 1, 0);
                check("addr_label_lo", label && str_eq(label, "lo"), 0);
            }
        }
    }
    expect("one_address", naddr, 1);
    expect("addr_dump_done", adone, 1);

    p_run("nobody_exit_0", 0, nobody_body);
    return failed ? 1 : 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    p_run("netns_child_exit_0", CLONE_NEWNET, body);
    return finish();
}
