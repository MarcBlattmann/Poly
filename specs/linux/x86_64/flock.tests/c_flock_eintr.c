/* flock: signals while sleeping. A caught signal whose handler lacks
 * SA_RESTART ends the sleep with EINTR; with SA_RESTART the call goes on
 * sleeping and returns 0 when the lock is granted. An ignored signal
 * does not end the sleep. There is no deadlock detection: a process
 * waits for a lock held by its own other open file.
 * poly-diff: syscalls=off (several processes)
 */

#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_flock 73
#define SYS_link 86
/* asm-generic/fcntl.h: O_PATH and the flock(2) operations */
#define O_PATH 010000000
#define LOCK_SH 1
#define LOCK_EX 2
#define LOCK_NB 4
#define LOCK_UN 8
#define LOCK_MAND 32
#define LOCK_READ 64
#define LOCK_WRITE 128

static __attribute__((unused)) i64 flock(i64 fd, i64 op) { return syscall2(SYS_flock, fd, op); }
static __attribute__((unused)) i64 open_at(const char *path, i64 flags) {
    return syscall4(SYS_openat, AT_FDCWD, path, flags, 0644);
}
static __attribute__((unused)) i64 unlink(const char *path) { return syscall1(SYS_unlink, path); }
static __attribute__((unused)) void exit_now(int code) {
    for (;;) syscall1(SYS_exit, code);
}

#define F "/tmp/c_flock_eintr"

static volatile int caught;
static void on_alarm(int sig) {
    (void)sig;
    caught++;
}

int test_main(void) {
    unlink(F);
    i64 a = open_at(F, O_RDWR | O_CREAT | O_TRUNC);
    i64 b = open_at(F, O_RDWR);
    check("open", a >= 0 && b >= 0, a);

    /* Self-conflict, ended by the handler. */
    check("handler", sig_handler(SIGALRM, on_alarm, 0) == 0, 0);
    check("b_ex", flock(b, LOCK_EX) == 0, 0);
    syscall1(SYS_alarm, 1);
    i64 t0 = now_ns();
    i64 r = flock(a, LOCK_EX);
    i64 ms = (now_ns() - t0) / 1000000;
    check("eintr", r == -EINTR, r);
    check("handler_ran", caught == 1, caught);
    check("slept_until_signal", ms >= 900 && ms < 5000, ms);
    r = flock(a, LOCK_SH | LOCK_NB);
    check("interrupted_took_nothing", r == -EAGAIN, r);

    /* Ignored signal: the sleep goes on past it, until a child (sharing
     * b's open file) unlocks half a second later. */
    check("ignore", sig_action(SIGALRM, 1, 0, 0) == 0, 0);
    i64 pid = fork();
    if (pid == 0) {
        sleep_ms(1500);
        exit_now(flock(b, LOCK_UN) == 0 ? 0 : 1);
    }
    close(b);
    syscall1(SYS_alarm, 1);
    t0 = now_ns();
    r = flock(a, LOCK_EX);
    ms = (now_ns() - t0) / 1000000;
    check("ignored_signal_no_eintr", r == 0, r);
    check("slept_past_signal", ms >= 1300, ms);
    check("child_unlocked", wait_exit(pid) == 0, 0);
    flock(a, LOCK_UN);

    /* SA_RESTART: the handler runs, the call goes on and succeeds. */
    i64 c = open_at(F, O_RDWR);
    check("c_ex", flock(c, LOCK_EX) == 0, 0);
    check("restart_handler", sig_handler(SIGALRM, on_alarm, SA_RESTART) == 0, 0);
    pid = fork();
    if (pid == 0) {
        sleep_ms(1500);
        exit_now(flock(c, LOCK_UN) == 0 ? 0 : 1);
    }
    close(c);
    syscall1(SYS_alarm, 1);
    r = flock(a, LOCK_EX);
    check("restarted_granted", r == 0, r);
    check("restart_handler_ran", caught == 2, caught);
    check("child2_unlocked", wait_exit(pid) == 0, 0);
    close(a);
    check("cleanup", unlink(F) == 0, 0);
    return finish();
}
