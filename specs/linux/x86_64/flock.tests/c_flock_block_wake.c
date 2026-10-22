/* flock: blocking. Without LOCK_NB a conflicting request sleeps until it
 * can be granted: it returns 0 once the holder unlocks, or once the
 * holder's last descriptor is closed. A sleeping exclusive request does
 * not keep new shared requests out (no writer preference).
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

#define F "/tmp/c_flock_block_wake"

/* 1 when `pid` has not exited yet. */
static int still_running(i64 pid) { return syscall4(SYS_wait4, pid, 0, WNOHANG, 0) == 0; }

int test_main(void) {
    unlink(F);
    i64 a = open_at(F, O_RDWR | O_CREAT | O_TRUNC);
    check("open", a >= 0, a);
    volatile unsigned *g = shared_page(4096);
    check("shared_page", g != 0, 0);

    /* Woken by LOCK_UN. */
    check("a_ex", flock(a, LOCK_EX) == 0, 0);
    i64 pid = fork();
    if (pid == 0) {
        close(a);
        i64 c = open_at(F, O_RDONLY);
        gate_open(&g[0]);
        i64 r = flock(c, LOCK_SH);
        if (r != 0) exit_now(1);
        /* Granted: the parent no longer holds it exclusively. */
        exit_now(__atomic_load_n(&g[1], __ATOMIC_SEQ_CST) == 1 ? 0 : 2);
    }
    gate_wait(&g[0]);
    sleep_ms(100);
    check("waiter_sleeps", still_running(pid), 0);
    __atomic_store_n(&g[1], 1, __ATOMIC_SEQ_CST);
    check("unlock", flock(a, LOCK_UN) == 0, 0);
    check("woken_by_unlock", wait_exit(pid) == 0, 0);

    /* Woken by the close of the holder's only descriptor. */
    check("a_sh", flock(a, LOCK_SH) == 0, 0);
    pid = fork();
    if (pid == 0) {
        close(a);
        i64 c = open_at(F, O_RDWR);
        gate_open(&g[2]);
        exit_now(flock(c, LOCK_EX) == 0 ? 0 : 1);
    }
    gate_wait(&g[2]);
    sleep_ms(100);
    check("ex_waiter_sleeps", still_running(pid), 0);
    /* A new shared request is granted while the exclusive one waits. */
    i64 s = open_at(F, O_RDONLY);
    i64 r = flock(s, LOCK_SH | LOCK_NB);
    check("shared_overtakes_waiting_ex", r == 0, r);
    close(s);
    sleep_ms(50);
    check("ex_waiter_still_sleeps", still_running(pid), 0);
    close(a);
    check("woken_by_close", wait_exit(pid) == 0, 0);
    check("cleanup", unlink(F) == 0, 0);
    return finish();
}
