/* flock: what is locked is the file (inode), not the path. Two hard
 * links are one file; an unlinked file stays lockable and keeps its
 * locks. The two ends of a pipe are one file; the two ends of a
 * socketpair are two. Two memfds are two files.
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

#define F "/tmp/c_flock_inode"
#define L "/tmp/c_flock_inode.link"
#define SYS_memfd_create 319

int test_main(void) {
    unlink(F);
    unlink(L);
    i64 a = open_at(F, O_RDWR | O_CREAT | O_TRUNC);
    check("open", a >= 0, a);
    i64 r = syscall2(SYS_link, F, L);
    check("link", r == 0, r);
    i64 l = open_at(L, O_RDONLY);
    check("a_ex", flock(a, LOCK_EX) == 0, 0);
    r = flock(l, LOCK_SH | LOCK_NB);
    check("hard_link_conflicts", r == -EAGAIN, r);
    check("unlink_both", unlink(F) == 0 && unlink(L) == 0, 0);
    r = flock(l, LOCK_SH | LOCK_NB);
    check("unlinked_still_locked", r == -EAGAIN, r);
    flock(a, LOCK_UN);
    r = flock(l, LOCK_EX | LOCK_NB);
    check("unlinked_lockable", r == 0, r);
    r = flock(a, LOCK_SH | LOCK_NB);
    check("unlinked_conflict", r == -EAGAIN, r);
    close(a);
    close(l);

    /* A path made again is a new file with no locks. */
    i64 n = open_at(F, O_RDWR | O_CREAT | O_TRUNC);
    r = flock(n, LOCK_EX | LOCK_NB);
    check("new_file_free", r == 0, r);
    close(n);
    unlink(F);

    int p[2];
    check("pipe", syscall1(SYS_pipe, p) == 0, 0);
    r = flock(p[0], LOCK_EX);
    check("pipe_read_end", r == 0, r);
    r = flock(p[1], LOCK_SH | LOCK_NB);
    check("pipe_ends_one_file", r == -EAGAIN, r);
    close(p[0]);
    r = flock(p[1], LOCK_EX | LOCK_NB);
    check("pipe_write_end_after_close", r == 0, r);
    close(p[1]);

    int sv[2];
    check("socketpair", syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, sv) == 0, 0);
    r = flock(sv[0], LOCK_EX);
    check("socket_0", r == 0, r);
    r = flock(sv[1], LOCK_EX | LOCK_NB);
    check("socket_1_separate", r == 0, r);
    close(sv[0]);
    close(sv[1]);

    i64 m1 = syscall2(SYS_memfd_create, "c_flock_a", 0);
    i64 m2 = syscall2(SYS_memfd_create, "c_flock_a", 0);
    r = flock(m1, LOCK_EX);
    check("memfd_1", r == 0, r);
    r = flock(m2, LOCK_EX | LOCK_NB);
    check("memfd_2_separate", r == 0, r);
    close(m1);
    close(m2);
    return finish();
}
