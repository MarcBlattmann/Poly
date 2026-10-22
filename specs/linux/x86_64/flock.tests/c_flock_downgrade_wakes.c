/* flock: one LOCK_EX -> LOCK_SH conversion wakes every sleeping shared
 * request at once; while they all hold LOCK_SH, LOCK_EX|LOCK_NB fails.
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

#define F "/tmp/c_flock_downgrade_wakes"
#define N 3

int test_main(void) {
    unlink(F);
    i64 a = open_at(F, O_RDWR | O_CREAT | O_TRUNC);
    i64 b = open_at(F, O_RDONLY);
    check("open", a >= 0 && b >= 0, a);
    volatile unsigned *g = shared_page(4096);
    check("shared_page", g != 0, 0);
    check("a_ex", flock(a, LOCK_EX) == 0, 0);
    i64 pid[N];
    for (int i = 0; i < N; i++) {
        pid[i] = fork();
        if (pid[i] == 0) {
            close(a);
            close(b);
            i64 c = open_at(F, O_RDONLY);
            __atomic_add_fetch(&g[0], 1, __ATOMIC_SEQ_CST);
            if (flock(c, LOCK_SH) != 0) exit_now(1);
            __atomic_add_fetch(&g[1], 1, __ATOMIC_SEQ_CST);
            gate_wait(&g[2]);
            exit_now(0);
        }
    }
    while (__atomic_load_n(&g[0], __ATOMIC_SEQ_CST) < N) sleep_ms(1);
    sleep_ms(100);
    check("none_granted_yet", __atomic_load_n(&g[1], __ATOMIC_SEQ_CST) == 0, g[1]);
    check("downgrade", flock(a, LOCK_SH) == 0, 0);
    for (int i = 0; i < 2000 && __atomic_load_n(&g[1], __ATOMIC_SEQ_CST) < N; i++) sleep_ms(1);
    check("all_granted", __atomic_load_n(&g[1], __ATOMIC_SEQ_CST) == N, g[1]);
    i64 r = flock(b, LOCK_EX | LOCK_NB);
    check("ex_vs_shared_holders", r == -EAGAIN, r);
    gate_open(&g[2]);
    int ok = 1;
    for (int i = 0; i < N; i++) ok &= wait_exit(pid[i]) == 0;
    check("children_ok", ok, 0);
    flock(a, LOCK_UN);
    r = flock(b, LOCK_EX | LOCK_NB);
    check("ex_after_all_left", r == 0, r);
    close(a);
    close(b);
    check("cleanup", unlink(F) == 0, 0);
    return finish();
}
