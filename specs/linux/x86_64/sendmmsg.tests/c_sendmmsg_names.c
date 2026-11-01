/* sendmmsg(2) on an unconnected UDP socket: every entry is sent to the
 * address in its own msg_name, so one call can reach several receivers;
 * an entry without a name fails with EDESTADDRREQ (the entries before
 * it are sent and counted).
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sendmmsg 307

struct k_mmsghdr {
    struct k_msghdr hdr;
    unsigned len;
    unsigned pad;
};

static i64 sendmmsg(i64 fd, struct k_mmsghdr *v, u64 n, u64 flags) { return syscall4(SYS_sendmmsg, fd, v, n, flags); }
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }

static struct k_mmsghdr mv[4];
static struct k_iovec iv[4];
static char text[4][4] = { "to1", "to2", "to1", "xxx" };

int test_main(void) {
    i64 r1 = syscall3(SYS_socket, AF_INET, SOCK_DGRAM, 0);
    i64 r2 = syscall3(SYS_socket, AF_INET, SOCK_DGRAM, 0);
    i64 s = syscall3(SYS_socket, AF_INET, SOCK_DGRAM, 0);
    check("sockets", r1 >= 0 && r2 >= 0 && s >= 0, r1);
    struct k_sockaddr_in a1 = in_loopback(0), a2 = in_loopback(0);
    expect("bind_1", syscall3(SYS_bind, r1, &a1, sizeof a1), 0);
    expect("bind_2", syscall3(SYS_bind, r2, &a2, sizeof a2), 0);
    int l = sizeof a1;
    expect("name_1", syscall3(SYS_getsockname, r1, &a1, &l), 0);
    l = sizeof a2;
    expect("name_2", syscall3(SYS_getsockname, r2, &a2, &l), 0);

    for (int i = 0; i < 4; i++) {
        iv[i].base = text[i];
        iv[i].len = 3;
        mv[i].hdr.iov = &iv[i];
        mv[i].hdr.iovlen = 1;
        mv[i].hdr.name = i == 1 ? &a2 : &a1;
        mv[i].hdr.namelen = sizeof a1;
    }
    mv[3].hdr.name = 0;
    mv[3].hdr.namelen = 0;
    expect("sent_3_then_no_name", sendmmsg(s, mv, 4, 0), 3);
    expect("first_no_name", sendmmsg(s, &mv[3], 1, 0), -EDESTADDRREQ);

    char buf[8];
    expect("r1_first", syscall6(SYS_recvfrom, r1, (i64)buf, 8, MSG_DONTWAIT, 0, 0), 3);
    check("r1_first_data", mem_eq(buf, "to1", 3), 0);
    expect("r1_second", syscall6(SYS_recvfrom, r1, (i64)buf, 8, MSG_DONTWAIT, 0, 0), 3);
    expect("r1_done", syscall6(SYS_recvfrom, r1, (i64)buf, 8, MSG_DONTWAIT, 0, 0), -EAGAIN);
    expect("r2_one", syscall6(SYS_recvfrom, r2, (i64)buf, 8, MSG_DONTWAIT, 0, 0), 3);
    check("r2_data", mem_eq(buf, "to2", 3), 0);
    expect("r2_done", syscall6(SYS_recvfrom, r2, (i64)buf, 8, MSG_DONTWAIT, 0, 0), -EAGAIN);

    close(r1);
    close(r2);
    close(s);
    return finish();
}
