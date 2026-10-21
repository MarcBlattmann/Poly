/* pselect6 argument checks: negative nfds is EINVAL; a negative tv_sec
 * or tv_nsec is EINVAL; a bad timespec pointer or a bad sigset pair
 * pointer is EFAULT; a bad sigset pointer inside the pair is EFAULT;
 * with nfds 0 and NULL sets it is a sleep of the timeout; a timespec is
 * written back with the time left. */
#include "poly_ipc.h"

struct sigset_arg {
    u64 *set;
    u64 size;
};

int test_main(void) {
    struct k_timespec zero = { 0, 0 };
    i64 r = syscall6(SYS_pselect6, -1, 0, 0, 0, (i64)&zero, 0);
    check("negative_nfds_einval", r == -EINVAL, r);
    struct k_timespec neg = { -1, 0 };
    r = syscall6(SYS_pselect6, 0, 0, 0, 0, (i64)&neg, 0);
    check("negative_sec_einval", r == -EINVAL, r);
    struct k_timespec negns = { 0, -5 };
    r = syscall6(SYS_pselect6, 0, 0, 0, 0, (i64)&negns, 0);
    check("negative_nsec_einval", r == -EINVAL, r);
    r = syscall6(SYS_pselect6, 0, 0, 0, 0, 8, 0);
    check("bad_timespec_efault", r == -EFAULT, r);
    r = syscall6(SYS_pselect6, 0, 0, 0, 0, (i64)&zero, 8);
    check("bad_pair_efault", r == -EFAULT, r);
    struct sigset_arg bad = { (u64 *)8, 8 };
    r = syscall6(SYS_pselect6, 0, 0, 0, 0, (i64)&zero, (i64)&bad);
    check("bad_sigset_efault", r == -EFAULT, r);

    struct k_timespec t = { 0, 20000000 };
    i64 t0 = now_ns();
    r = syscall6(SYS_pselect6, 0, 0, 0, 0, (i64)&t, 0);
    check("sleep_returns_zero", r == 0 && now_ns() - t0 >= 19000000, r);
    check("left_zero", t.tv_sec == 0 && t.tv_nsec == 0, t.tv_nsec);
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    struct k_fd_set wr;
    fdset_zero(&wr);
    fdset_add(&wr, p[1]);
    struct k_timespec ten = { 10, 0 };
    r = syscall6(SYS_pselect6, p[1] + 1, 0, (i64)&wr, 0, (i64)&ten, 0);
    check("ready_at_once", r == 1, r);
    check("left_written_back", ten.tv_sec == 9 || ten.tv_sec == 10, ten.tv_sec);
    return finish();
}
