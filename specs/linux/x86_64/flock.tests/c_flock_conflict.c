/* flock: conflicts between two open files of one file in one process.
 * Locks belong to the open file, not to the process: a second open of
 * the same file conflicts with the first exactly as another process
 * would. Shared + shared is fine; exclusive conflicts with both; with
 * LOCK_NB a conflict is EAGAIN (EWOULDBLOCK) and changes nothing.
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

#define F "/tmp/c_flock_conflict"

int test_main(void) {
    unlink(F);
    i64 a = open_at(F, O_RDWR | O_CREAT | O_TRUNC);
    i64 b = open_at(F, O_RDWR);
    i64 c = open_at(F, O_RDONLY);
    check("open", a >= 0 && b >= 0 && c >= 0, a);

    check("a_sh", flock(a, LOCK_SH) == 0, 0);
    i64 r = flock(b, LOCK_SH | LOCK_NB);
    check("sh_sh_ok", r == 0, r);
    r = flock(c, LOCK_SH);
    check("third_sh_ok", r == 0, r);
    r = flock(c, LOCK_EX | LOCK_NB);
    check("ex_vs_two_sh_eagain", r == -EAGAIN, r);
    flock(b, LOCK_UN);
    flock(c, LOCK_UN);
    r = flock(b, LOCK_EX | LOCK_NB);
    check("ex_vs_sh_eagain", r == -EAGAIN, r);
    check("a_un", flock(a, LOCK_UN) == 0, 0);
    r = flock(b, LOCK_EX | LOCK_NB);
    check("ex_after_unlock", r == 0, r);
    r = flock(a, LOCK_SH | LOCK_NB);
    check("sh_vs_ex_eagain", r == -EAGAIN, r);
    r = flock(a, LOCK_EX | LOCK_NB);
    check("ex_vs_ex_eagain", r == -EAGAIN, r);
    r = flock(c, LOCK_SH | LOCK_NB);
    check("third_sh_vs_ex_eagain", r == -EAGAIN, r);
    /* LOCK_UN on an open file that holds nothing leaves b's lock alone. */
    r = flock(a, LOCK_UN);
    check("unlock_without_lock", r == 0, r);
    r = flock(c, LOCK_SH | LOCK_NB);
    check("other_lock_kept", r == -EAGAIN, r);
    /* The holder itself may relock in any mode. */
    r = flock(b, LOCK_EX | LOCK_NB);
    check("holder_relock", r == 0, r);
    check("b_un", flock(b, LOCK_UN) == 0, 0);
    r = flock(a, LOCK_EX | LOCK_NB);
    check("ex_after_release", r == 0, r);
    flock(a, LOCK_UN);
    close(a);
    close(b);
    close(c);
    check("cleanup", unlink(F) == 0, 0);
    return finish();
}
