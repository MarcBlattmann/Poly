/* kcmp(pid1, pid2, KCMP_FILE, fd1, fd2) compares the open file
 * descriptions behind fd1 of pid1 and fd2 of pid2: 0 when they are the
 * same (dup, dup2, inherited by a child), otherwise 1 or 2, an ordering
 * that is consistent (swapping the descriptors gives the other value) but
 * not meaningful. Two opens of the same file are different
 * descriptions. A descriptor that is not open is EBADF; only the low 32
 * bits of fd1/fd2 count.
 * poly-diff: syscalls=names (pids are arguments)
 */
#include "a_sys.h"

#define SYS_kcmp 312
#define KCMP_FILE 0 /* linux/kcmp.h */

static i64 kcmp(i64 p1, i64 p2, i64 type, i64 i1, i64 i2) { return syscall5(SYS_kcmp, p1, p2, type, i1, i2); }

static volatile i32 done, cmd;
static volatile i64 res;
static int child(void *unused) {
    (void)unused;
    while (!cmd) a_relax();
    /* Replace the copy of the parent's fd 4 by a new open of the file. */
    a_close(4);
    res = a_open("/tmp/ld_kcmp_file", O_RDONLY, 0);
    cmd = 0;
    while (!cmd) a_relax();
    cmd = 0;
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    i64 me = syscall0(SYS_getpid);
    i64 a = a_open("/tmp/ld_kcmp_file", O_RDWR | O_CREAT | O_TRUNC, 0600);
    i64 b = syscall1(SYS_dup, a);
    i64 c = a_open("/tmp/ld_kcmp_file", O_RDWR, 0);
    expect("fds", a * 100 + b * 10 + c, 345);
    expect("same_fd", kcmp(me, me, KCMP_FILE, a, a), 0);
    expect("dup_same", kcmp(me, me, KCMP_FILE, a, b), 0);
    i64 ac = kcmp(me, me, KCMP_FILE, a, c), ca = kcmp(me, me, KCMP_FILE, c, a);
    check("second_open_differs", ac == 1 || ac == 2, ac);
    check("order_consistent", ac + ca == 3, ca);
    check("dup_orders_alike", kcmp(me, me, KCMP_FILE, b, c) == ac, 0);
    expect("dup2", syscall2(SYS_dup2, c, 9), 9);
    expect("dup2_same", kcmp(me, me, KCMP_FILE, 9, c), 0);
    expect("fd_not_open_ebadf", kcmp(me, me, KCMP_FILE, a, 20), -EBADF);
    expect("fd1_not_open_ebadf", kcmp(me, me, KCMP_FILE, 20, a), -EBADF);
    expect("fd_minus1_ebadf", kcmp(me, me, KCMP_FILE, -1, a), -EBADF);
    expect("fd_upper_half_ignored", kcmp(me, me, KCMP_FILE, (1L << 32) | a, b), 0);

    /* A child's copy of the descriptor table refers to the same files. */
    i64 pid = a_spawn(0, &done, child, 0);
    expect("child_fd3_same", kcmp(me, pid, KCMP_FILE, a, a), 0);
    expect("child_fd4_is_parent_fd3", kcmp(pid, me, KCMP_FILE, b, a), 0);
    cmd = 1;
    while (cmd) a_relax();
    expect("child_reopened_fd4", res, 4);
    i64 x = kcmp(pid, me, KCMP_FILE, 4, b);
    check("child_new_open_differs", x == 1 || x == 2, x);
    expect("child_fd5_ebadf_free", kcmp(pid, me, KCMP_FILE, 12, a), -EBADF);
    cmd = 2;
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    syscall4(SYS_wait4, pid, 0, 0, 0);
    a_unlink("/tmp/ld_kcmp_file");
    return finish();
}
