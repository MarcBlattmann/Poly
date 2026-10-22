/* flock: flock locks and fcntl (POSIX record) locks are independent. An
 * open file holding LOCK_EX can still take a whole-file F_WRLCK, and
 * another process sees each kind conflict only with its own kind.
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

#define F "/tmp/c_flock_fcntl"
/* asm-generic/fcntl.h */
#define F_GETLK 5
#define F_SETLK 6
#define F_RDLCK 0
#define F_WRLCK 1
#define F_UNLCK 2
#define SEEK_SET 0

struct k_flock {
    short l_type;
    short l_whence;
    i64 l_start;
    i64 l_len;
    int l_pid;
};

int test_main(void) {
    unlink(F);
    i64 a = open_at(F, O_RDWR | O_CREAT | O_TRUNC);
    check("open", a >= 0, a);
    check("flock_ex", flock(a, LOCK_EX) == 0, 0);
    struct k_flock k = { F_WRLCK, SEEK_SET, 0, 0, 0 };
    i64 r = syscall3(SYS_fcntl, a, F_SETLK, &k);
    check("fcntl_wrlck_while_flock_ex", r == 0, r);

    i64 pid = fork();
    if (pid == 0) {
        int bad = 0;
        i64 c = open_at(F, O_RDWR);
        struct k_flock q = { F_RDLCK, SEEK_SET, 0, 0, 0 };
        if (syscall3(SYS_fcntl, c, F_SETLK, &q) != -EAGAIN) bad |= 1;
        if (flock(c, LOCK_SH | LOCK_NB) != -EAGAIN) bad |= 2;
        exit_now(bad);
    }
    check("child_sees_both", wait_exit(pid) == 0, 0);

    /* Drop only the flock lock: the record lock stays. */
    check("flock_un", flock(a, LOCK_UN) == 0, 0);
    pid = fork();
    if (pid == 0) {
        int bad = 0;
        i64 c = open_at(F, O_RDWR);
        struct k_flock q = { F_WRLCK, SEEK_SET, 0, 0, 0 };
        if (syscall3(SYS_fcntl, c, F_SETLK, &q) != -EAGAIN) bad |= 1;
        if (flock(c, LOCK_EX | LOCK_NB) != 0) bad |= 2;
        exit_now(bad);
    }
    check("record_lock_only", wait_exit(pid) == 0, 0);

    /* Drop the record lock, take flock again: only flock conflicts. */
    k.l_type = F_UNLCK;
    check("fcntl_unlck", syscall3(SYS_fcntl, a, F_SETLK, &k) == 0, 0);
    check("flock_sh", flock(a, LOCK_SH) == 0, 0);
    pid = fork();
    if (pid == 0) {
        int bad = 0;
        i64 c = open_at(F, O_RDWR);
        struct k_flock q = { F_WRLCK, SEEK_SET, 0, 0, 0 };
        if (syscall3(SYS_fcntl, c, F_SETLK, &q) != 0) bad |= 1;
        if (flock(c, LOCK_EX | LOCK_NB) != -EAGAIN) bad |= 2;
        if (flock(c, LOCK_SH | LOCK_NB) != 0) bad |= 4;
        exit_now(bad);
    }
    check("flock_only", wait_exit(pid) == 0, 0);
    close(a);
    check("cleanup", unlink(F) == 0, 0);
    return finish();
}
