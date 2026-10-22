/* flock: the three operations on one regular file. LOCK_SH, LOCK_EX and
 * LOCK_UN return 0, also with LOCK_NB, also when the open file already
 * holds that lock or (LOCK_UN) holds none. The access mode of the open
 * file does not matter: a read-only or write-only file takes either lock.
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

#define F "/tmp/c_flock_basic"

int test_main(void) {
    unlink(F);
    i64 rw = open_at(F, O_RDWR | O_CREAT | O_TRUNC);
    check("open_rw", rw >= 0, rw);
    i64 r = flock(rw, LOCK_UN);
    check("unlock_unlocked", r == 0, r);
    r = flock(rw, LOCK_SH);
    check("shared", r == 0, r);
    r = flock(rw, LOCK_SH);
    check("shared_again", r == 0, r);
    r = flock(rw, LOCK_EX);
    check("exclusive", r == 0, r);
    r = flock(rw, LOCK_EX | LOCK_NB);
    check("exclusive_again_nb", r == 0, r);
    r = flock(rw, LOCK_UN);
    check("unlock", r == 0, r);
    r = flock(rw, LOCK_UN | LOCK_NB);
    check("unlock_nb", r == 0, r);
    r = flock(rw, LOCK_SH | LOCK_NB);
    check("shared_nb", r == 0, r);
    r = flock(rw, LOCK_UN);
    check("unlock_2", r == 0, r);
    close(rw);

    i64 ro = open_at(F, O_RDONLY);
    r = flock(ro, LOCK_EX);
    check("rdonly_exclusive", r == 0, r);
    r = flock(ro, LOCK_SH);
    check("rdonly_shared", r == 0, r);
    close(ro);
    i64 wo = open_at(F, O_WRONLY);
    r = flock(wo, LOCK_SH);
    check("wronly_shared", r == 0, r);
    r = flock(wo, LOCK_EX);
    check("wronly_exclusive", r == 0, r);
    close(wo);
    /* Closing released the lock: a new open file takes it at once. */
    i64 again = open_at(F, O_RDONLY);
    r = flock(again, LOCK_EX | LOCK_NB);
    check("free_after_close", r == 0, r);
    close(again);
    check("cleanup", unlink(F) == 0, 0);
    return finish();
}
