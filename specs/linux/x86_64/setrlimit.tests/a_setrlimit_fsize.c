/* setrlimit RLIMIT_FSIZE: a write that would take a file past the limit
 * is cut short at it, and one starting at it fails with EFBIG and raises
 * SIGXFSZ (blocked here, so it stays pending). */
#include "a_sys.h"

#define F "/tmp/a_setrlimit_fsize"
#define SIGXFSZ 25

int test_main(void) {
    struct a_rlimit old, l;
    u64 block = A_SIGBIT(SIGXFSZ);
    syscall4(SYS_rt_sigprocmask, SIG_BLOCK, &block, 0, 8);
    syscall2(SYS_getrlimit, RLIMIT_FSIZE, &old);
    l.rlim_cur = 10;
    l.rlim_max = old.rlim_max;
    expect("set_fsize_10", syscall2(SYS_setrlimit, RLIMIT_FSIZE, &l), 0);
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    expect("write_within", a_write(fd, "123456", 6), 6);
    u64 pending = 0;
    syscall2(SYS_rt_sigpending, &pending, 8);
    expect("no_signal_yet", pending & block, 0);
    expect("write_cut_at_limit", a_write(fd, "abcdefgh", 8), 4);
    expect("write_at_limit_efbig", a_write(fd, "x", 1), -EFBIG);
    syscall2(SYS_rt_sigpending, &pending, 8);
    expect("sigxfsz_pending", pending & block, block);
    expect("pwrite_past_limit_efbig", syscall4(SYS_pwrite64, fd, "y", 1, 20), -EFBIG);
    expect("ftruncate_past_limit_efbig", syscall2(SYS_ftruncate, fd, 11), -EFBIG);
    expect("ftruncate_to_limit_ok", syscall2(SYS_ftruncate, fd, 10), 0);
    struct a_stat st;
    syscall2(SYS_fstat, fd, &st);
    expect("size_is_limit", st.st_size, 10);
    a_close(fd);
    syscall2(SYS_setrlimit, RLIMIT_FSIZE, &old);
    /* Take the pending SIGXFSZ without running a handler. */
    struct a_siginfo si;
    struct a_timespec zero = {0, 0};
    expect("accept_sigxfsz", syscall4(SYS_rt_sigtimedwait, &block, &si, &zero, 8), SIGXFSZ);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
