/* A blocking recvmmsg(2) without MSG_WAITFORONE waits until vlen
 * messages have arrived (two datagrams sent 100 ms apart: both are
 * returned in one call). With MSG_WAITFORONE it returns after the first.
 * While no message at all has arrived the timeout is not looked at: a
 * call with a 50 ms timeout still waits for the first message (sent
 * after 200 ms) and then returns. A signal with a handler while nothing
 * was received ends the call with EINTR.
 * poly-diff: syscalls=off (two processes, timing)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_recvmmsg 299
/* linux/socket.h */
#define MSG_WAITFORONE 0x10000

struct k_mmsghdr {
    struct k_msghdr hdr;
    unsigned len;
    unsigned pad;
};

static i64 recvmmsg(i64 fd, struct k_mmsghdr *v, u64 n, u64 flags, void *ts) {
    return syscall5(SYS_recvmmsg, fd, v, n, flags, ts);
}
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }
static volatile int got_usr1;
static void on_usr1(int s) {
    (void)s;
    got_usr1++;
}

static struct k_mmsghdr mv[2];
static struct k_iovec iv[2];
static char bufs[2][16];

int test_main(void) {
    int d[2];
    expect("socketpair", syscall4(SYS_socketpair, AF_UNIX, SOCK_DGRAM, 0, d), 0);
    for (int i = 0; i < 2; i++) {
        iv[i].base = bufs[i];
        iv[i].len = 16;
        mv[i].hdr.iov = &iv[i];
        mv[i].hdr.iovlen = 1;
    }
    i64 parent = getpid();

    i64 pid = fork();
    if (pid == 0) {
        sleep_ms(100);
        write(d[0], "first", 5);
        sleep_ms(100);
        write(d[0], "second", 6);
        syscall1(SYS_exit, 0);
    }
    expect("waits_for_both", recvmmsg(d[1], mv, 2, 0, 0), 2);
    check("both_lens", mv[0].len == 5 && mv[1].len == 6, mv[1].len);
    expect("child_1", wait_exit(pid), 0);

    pid = fork();
    if (pid == 0) {
        sleep_ms(100);
        write(d[0], "first", 5);
        sleep_ms(300);
        write(d[0], "second", 6);
        syscall1(SYS_exit, 0);
    }
    expect("waitforone_returns_1", recvmmsg(d[1], mv, 2, MSG_WAITFORONE, 0), 1);
    expect("child_2", wait_exit(pid), 0);
    expect("second_later", recvmmsg(d[1], mv, 2, MSG_DONTWAIT, 0), 1);

    pid = fork();
    if (pid == 0) {
        sleep_ms(200);
        write(d[0], "late", 4);
        syscall1(SYS_exit, 0);
    }
    struct k_timespec ts = { 0, 50000000 };
    i64 t0 = now_ns();
    expect("timeout_ignored_until_first", recvmmsg(d[1], mv, 2, 0, &ts), 1);
    check("waited_past_timeout", now_ns() - t0 >= 150000000L, 0);
    expect("child_3", wait_exit(pid), 0);

    expect("handler", sig_handler(SIGUSR1, on_usr1, 0), 0);
    pid = fork();
    if (pid == 0) {
        sleep_ms(100);
        syscall2(SYS_kill, parent, SIGUSR1);
        syscall1(SYS_exit, 0);
    }
    expect("eintr", recvmmsg(d[1], mv, 2, 0, 0), -EINTR);
    expect("handler_ran", got_usr1, 1);
    expect("child_4", wait_exit(pid), 0);

    close(d[0]);
    close(d[1]);
    return finish();
}
