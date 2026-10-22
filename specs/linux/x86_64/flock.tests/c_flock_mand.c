/* flock: LOCK_MAND (32). Any operation with bit 32 set returns 0 and does
 * nothing: no lock is taken, none is released, other bits are not
 * checked, and neither is the descriptor (closed, negative, O_PATH).
 * An operation of -1 (all bits set) is such an operation.
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

#define F "/tmp/c_flock_mand"

int test_main(void) {
    unlink(F);
    i64 a = open_at(F, O_RDWR | O_CREAT | O_TRUNC);
    i64 b = open_at(F, O_RDONLY);
    check("open", a >= 0 && b >= 0, a);
    i64 r = flock(a, LOCK_MAND);
    check("mand_alone", r == 0, r);
    r = flock(a, LOCK_MAND | LOCK_READ);
    check("mand_read", r == 0, r);
    r = flock(a, LOCK_MAND | LOCK_READ | LOCK_WRITE);
    check("mand_read_write", r == 0, r);
    r = flock(a, LOCK_MAND | LOCK_SH | LOCK_EX);
    check("mand_sh_ex", r == 0, r);
    r = flock(a, LOCK_MAND | 256);
    check("mand_256", r == 0, r);
    r = flock(a, -1);
    check("all_bits", r == 0, r);
    r = flock(99, LOCK_MAND);
    check("mand_closed_fd", r == 0, r);
    r = flock(-1, LOCK_MAND | LOCK_EX);
    check("mand_negative_fd", r == 0, r);
    i64 p = open_at(F, O_PATH);
    r = flock(p, LOCK_MAND | LOCK_EX);
    check("mand_opath_fd", r == 0, r);

    /* LOCK_MAND|LOCK_EX took nothing. */
    r = flock(a, LOCK_MAND | LOCK_EX);
    check("mand_ex", r == 0, r);
    r = flock(b, LOCK_EX | LOCK_NB);
    check("mand_ex_took_nothing", r == 0, r);
    /* While b holds LOCK_EX, a conflicting LOCK_MAND call still succeeds... */
    r = flock(a, LOCK_MAND | LOCK_SH | LOCK_NB);
    check("mand_sh_nb_against_ex", r == 0, r);
    r = flock(a, LOCK_MAND | LOCK_SH);
    check("mand_sh_against_ex_no_wait", r == 0, r);
    /* ...and LOCK_MAND|LOCK_UN on b releases nothing. */
    r = flock(b, LOCK_MAND | LOCK_UN);
    check("mand_un", r == 0, r);
    r = flock(a, LOCK_SH | LOCK_NB);
    check("mand_un_released_nothing", r == -EAGAIN, r);
    flock(b, LOCK_UN);
    r = flock(a, LOCK_SH | LOCK_NB);
    check("real_unlock_releases", r == 0, r);
    close(p);
    close(a);
    close(b);
    check("cleanup", unlink(F) == 0, 0);
    return finish();
}
