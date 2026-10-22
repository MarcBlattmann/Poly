/* flock: the lock belongs to the open file description. Descriptors made
 * by dup share it: either can unlock it, and relocking through the other
 * one is no conflict. Closing one of them keeps the lock; closing the
 * last descriptor of the open file releases it.
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

#define F "/tmp/c_flock_dup_close"

int test_main(void) {
    unlink(F);
    i64 a = open_at(F, O_RDWR | O_CREAT | O_TRUNC);
    i64 other = open_at(F, O_RDONLY);
    i64 d = syscall1(SYS_dup, a);
    check("open_dup", a >= 0 && other >= 0 && d >= 0, d);

    check("a_ex", flock(a, LOCK_EX) == 0, 0);
    i64 r = flock(d, LOCK_EX | LOCK_NB);
    check("dup_relock_no_conflict", r == 0, r);
    r = flock(d, LOCK_SH);
    check("dup_converts_shared", r == 0, r);
    r = flock(other, LOCK_SH | LOCK_NB);
    check("conversion_seen_by_others", r == 0, r);
    flock(other, LOCK_UN);
    check("a_ex_again", flock(a, LOCK_EX) == 0, 0);
    r = flock(d, LOCK_UN);
    check("dup_unlocks", r == 0, r);
    r = flock(other, LOCK_EX | LOCK_NB);
    check("released_through_dup", r == 0, r);
    flock(other, LOCK_UN);

    check("a_ex_3", flock(a, LOCK_EX) == 0, 0);
    close(a);
    r = flock(other, LOCK_SH | LOCK_NB);
    check("close_one_keeps", r == -EAGAIN, r);
    i64 d2 = syscall2(SYS_dup2, d, 20);
    close(d);
    r = flock(other, LOCK_SH | LOCK_NB);
    check("dup2_keeps", r == -EAGAIN && d2 == 20, r);
    close(20);
    r = flock(other, LOCK_EX | LOCK_NB);
    check("close_last_releases", r == 0, r);

    /* Opening the path again makes a new open file, which conflicts. */
    i64 again = open_at(F, O_RDONLY);
    r = flock(again, LOCK_SH | LOCK_NB);
    check("reopen_conflicts", r == -EAGAIN, r);
    close(other);
    r = flock(again, LOCK_SH | LOCK_NB);
    check("reopen_free", r == 0, r);
    close(again);
    check("cleanup", unlink(F) == 0, 0);
    return finish();
}
