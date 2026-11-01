/* A blocking splice(2) waits: from an empty pipe until a writer puts
 * bytes in (then returns what arrived), into a full pipe until a reader
 * makes room. A signal with a handler ends the wait with EINTR when the
 * handler lacks SA_RESTART; with SA_RESTART the call is restarted and
 * completes once data arrives.
 * poly-diff: syscalls=off (two processes, timing)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_splice 275

#define F "/tmp/c_splice_blocking"

static i64 splice(i64 in, i64 *offin, i64 out, i64 *offout, u64 len, u64 flags) {
    return syscall6(SYS_splice, in, (i64)offin, out, (i64)offout, (i64)len, (i64)flags);
}
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }
static volatile int got_usr1;
static void on_usr1(int s) {
    (void)s;
    got_usr1++;
}

int test_main(void) {
    i64 f = syscall4(SYS_openat, AT_FDCWD, F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    check("open", f >= 0, f);
    int p[2];
    expect("pipe", syscall1(SYS_pipe, p), 0);
    i64 parent = getpid();

    /* Empty pipe -> file: waits for the writer. */
    i64 pid = fork();
    if (pid == 0) {
        sleep_ms(100);
        write(p[1], "late!", 5);
        syscall1(SYS_exit, 0);
    }
    i64 off = 0;
    expect("waits_for_data", splice(p[0], 0, f, &off, 100, 0), 5);
    expect("off_5", off, 5);
    expect("child_1", wait_exit(pid), 0);

    /* Full pipe: waits for room. */
    i64 zero = syscall4(SYS_openat, AT_FDCWD, "/dev/zero", O_RDONLY, 0);
    expect("nb", syscall3(SYS_fcntl, p[1], F_SETFL, O_NONBLOCK), 0);
    expect("fill", splice(zero, 0, p[1], 0, 1 << 20, 0), 65536);
    expect("block", syscall3(SYS_fcntl, p[1], F_SETFL, 0), 0);
    pid = fork();
    if (pid == 0) {
        sleep_ms(100);
        static char page[4096];
        read(p[0], page, sizeof page); /* frees one whole buffer */
        syscall1(SYS_exit, 0);
    }
    expect("waits_for_room", splice(zero, 0, p[1], 0, 10, 0), 10);
    expect("child_2", wait_exit(pid), 0);
    i64 null = syscall4(SYS_openat, AT_FDCWD, "/dev/null", O_WRONLY, 0);
    while (splice(p[0], 0, null, 0, 1 << 20, 2) > 0) {
    }

    /* EINTR without SA_RESTART. */
    expect("handler", sig_handler(SIGUSR1, on_usr1, 0), 0);
    pid = fork();
    if (pid == 0) {
        sleep_ms(100);
        syscall2(SYS_kill, parent, SIGUSR1);
        syscall1(SYS_exit, 0);
    }
    expect("eintr", splice(p[0], 0, f, &off, 10, 0), -EINTR);
    expect("handler_ran", got_usr1, 1);
    expect("child_3", wait_exit(pid), 0);

    /* SA_RESTART: restarted, then the data arrives. */
    expect("handler_restart", sig_handler(SIGUSR1, on_usr1, SA_RESTART), 0);
    pid = fork();
    if (pid == 0) {
        sleep_ms(100);
        syscall2(SYS_kill, parent, SIGUSR1);
        sleep_ms(100);
        write(p[1], "abc", 3);
        syscall1(SYS_exit, 0);
    }
    expect("restarted", splice(p[0], 0, f, &off, 10, 0), 3);
    expect("handler_ran_2", got_usr1, 2);
    expect("child_4", wait_exit(pid), 0);

    close(zero);
    close(null);
    close(p[0]);
    close(p[1]);
    close(f);
    expect("unlink", syscall1(SYS_unlink, F), 0);
    return finish();
}
