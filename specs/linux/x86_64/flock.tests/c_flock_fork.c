/* flock: locks across fork and exit. A child's inherited descriptor is
 * the same open file, so it shares the parent's lock: the child can
 * unlock it, and a lock the child takes through it outlives the child.
 * A lock on an open file only the child had is released when the child
 * exits, also when it is killed by SIGKILL.
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

#define F "/tmp/c_flock_fork"

int test_main(void) {
    unlink(F);
    i64 a = open_at(F, O_RDWR | O_CREAT | O_TRUNC);
    i64 b = open_at(F, O_RDONLY);
    check("open", a >= 0 && b >= 0, a);
    volatile unsigned *g = shared_page(4096);
    check("shared_page", g != 0, 0);

    /* The child sees the parent's lock as its own. */
    check("a_ex", flock(a, LOCK_EX) == 0, 0);
    i64 pid = fork();
    if (pid == 0) {
        int bad = 0;
        if (flock(a, LOCK_EX | LOCK_NB) != 0) bad |= 1;
        if (flock(b, LOCK_SH | LOCK_NB) != -EAGAIN) bad |= 2;
        if (flock(a, LOCK_UN) != 0) bad |= 4;
        exit_now(bad);
    }
    check("child_shares_lock", wait_exit(pid) == 0, 0);
    i64 r = flock(b, LOCK_EX | LOCK_NB);
    check("child_unlocked_parents_lock", r == 0, r);
    flock(b, LOCK_UN);

    /* A lock the child takes through the inherited descriptor stays. */
    pid = fork();
    if (pid == 0) exit_now(flock(a, LOCK_EX | LOCK_NB) == 0 ? 0 : 1);
    check("child_locks_inherited", wait_exit(pid) == 0, 0);
    r = flock(b, LOCK_SH | LOCK_NB);
    check("lock_outlives_child", r == -EAGAIN, r);
    flock(a, LOCK_UN);

    /* A child's own open file: released at exit. */
    pid = fork();
    if (pid == 0) {
        close(a);
        close(b);
        i64 c = open_at(F, O_RDONLY);
        exit_now(flock(c, LOCK_EX | LOCK_NB) == 0 ? 0 : 1);
    }
    check("child_own_lock", wait_exit(pid) == 0, 0);
    r = flock(b, LOCK_EX | LOCK_NB);
    check("released_at_exit", r == 0, r);
    flock(b, LOCK_UN);

    /* And when killed. */
    pid = fork();
    if (pid == 0) {
        close(a);
        close(b);
        i64 c = open_at(F, O_RDONLY);
        if (flock(c, LOCK_EX) != 0) exit_now(1);
        gate_open(&g[0]);
        for (;;) sleep_ms(1000);
    }
    gate_wait(&g[0]);
    r = flock(b, LOCK_SH | LOCK_NB);
    check("held_by_live_child", r == -EAGAIN, r);
    syscall2(SYS_kill, pid, SIGKILL);
    int st = wait_status(pid);
    check("child_killed", W_SIGNALED(st) && W_TERMSIG(st) == SIGKILL, st);
    r = flock(b, LOCK_SH | LOCK_NB);
    check("released_at_kill", r == 0, r);
    close(a);
    close(b);
    check("cleanup", unlink(F) == 0, 0);
    return finish();
}
