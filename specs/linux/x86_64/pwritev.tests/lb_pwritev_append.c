/* pwritev(2) on a descriptor opened with O_APPEND: Linux appends the data
 * at the end of the file whatever offset is given; the file position is
 * not used or moved. RLIMIT_FSIZE: a write that would cross the limit is
 * cut short at it, one that starts at it fails with EFBIG (and SIGXFSZ). */
#include "a_sys.h"

#define D "/tmp/lb_pwritev_append"
#define SIGXFSZ 25

static i64 pwritev_(i64 fd, struct a_iovec *iov, i64 cnt, i64 lo) { return syscall5(SYS_pwritev, fd, iov, cnt, lo, 0); }

int test_main(void) {
    char back[64];
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "0123", 0644), 0);
    i64 ap = a_open(D "/f", O_WRONLY | O_APPEND, 0);
    struct a_iovec v[2] = {{"ab", 2}, {"c", 1}};
    expect("append_ignores_offset", pwritev_(ap, v, 2, 0), 3);
    expect("position_still_0", syscall3(SYS_lseek, ap, 0, SEEK_CUR), 0);
    i64 r = a_open(D "/f", O_RDONLY, 0);
    expect("read_back", a_read(r, back, 64), 7);
    expect("appended", mem_eq(back, "0123abc", 7), 1);
    expect("append_offset_far", pwritev_(ap, v, 1, 1000), 2);
    expect("size_9", syscall2(SYS_fstat, ap, &st) == 0 ? st.st_size : -1, 9);

    /* RLIMIT_FSIZE */
    i64 fd = a_open(D "/g", O_RDWR | O_CREAT, 0644);
    struct a_sigaction ign = {SIG_IGN, 0, 0, 0};
    expect("ignore_sigxfsz", syscall4(SYS_rt_sigaction, SIGXFSZ, &ign, 0, 8), 0);
    /* the limit applies to every regular file, the test's output too:
     * collect the results first, check them after lifting the limit */
    struct a_rlimit lim = {10, RLIM_INFINITY}, unlimited = {RLIM_INFINITY, RLIM_INFINITY};
    struct a_iovec big[2] = {{"abcdef", 6}, {"ghijkl", 6}};
    i64 set = syscall2(SYS_setrlimit, RLIMIT_FSIZE, &lim);
    i64 cut = pwritev_(fd, big, 2, 0);
    i64 at = pwritev_(fd, big, 1, 10);
    i64 below = pwritev_(fd, big, 1, 2);
    i64 unset = syscall2(SYS_setrlimit, RLIMIT_FSIZE, &unlimited);
    expect("setrlimit", set, 0);
    expect("cut_at_limit", cut, 10);
    expect("at_limit_efbig", at, -EFBIG);
    expect("below_limit_ok", below, 6);
    expect("unset_rlimit", unset, 0);
    expect("size_10", syscall2(SYS_fstat, fd, &st) == 0 ? st.st_size : -1, 10);
    a_close(fd);

    a_close(r);
    a_close(ap);
    expect("cleanup_g", a_unlink(D "/g"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
