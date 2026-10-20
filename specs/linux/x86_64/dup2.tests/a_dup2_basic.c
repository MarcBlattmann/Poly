/* dup2: onto a free number, onto an open number (closed first), onto
 * itself, and its errors. */
#include "a_sys.h"

#define F "/tmp/a_dup2_basic"

int test_main(void) {
    char c;
    expect("setup_file", a_make_file(F, "abc", 0644), 0);
    i64 fd = a_open(F, O_RDONLY | O_CLOEXEC, 0);
    expect("dup2_to_free_10", syscall2(SYS_dup2, fd, 10), 10);
    expect("target_no_cloexec", syscall3(SYS_fcntl, 10, F_GETFD, 0), 0);
    a_read(10, &c, 1);
    expect("target_shares_offset", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 1);
    expect("next_open_is_4", a_open(F, O_RDONLY, 0), 4);
    /* Onto 4: the file open there is closed first. */
    expect("dup2_onto_open_4", syscall2(SYS_dup2, fd, 4), 4);
    expect("old_4_replaced_offset", syscall3(SYS_lseek, 4, 0, SEEK_CUR), 1);
    /* Onto itself: nothing changes, flags stay. */
    expect("dup2_self", syscall2(SYS_dup2, fd, fd), fd);
    expect("self_keeps_cloexec", syscall3(SYS_fcntl, fd, F_GETFD, 0), FD_CLOEXEC);
    expect("dup2_self_closed_ebadf", syscall2(SYS_dup2, 20, 20), -EBADF);
    expect("dup2_bad_old_ebadf", syscall2(SYS_dup2, 21, 5), -EBADF);
    expect("bad_old_left_target_free", syscall3(SYS_fcntl, 5, F_GETFD, 0), -EBADF);
    expect("dup2_bad_old_keeps_target", syscall2(SYS_dup2, 22, 4), -EBADF);
    expect("target_4_still_open", syscall3(SYS_fcntl, 4, F_GETFD, 0), 0);
    expect("dup2_negative_new_ebadf", syscall2(SYS_dup2, fd, -1), -EBADF);
    struct a_rlimit old, lim;
    syscall2(SYS_getrlimit, RLIMIT_NOFILE, &old);
    lim.rlim_cur = 64;
    lim.rlim_max = old.rlim_max;
    expect("set_limit_64", syscall2(SYS_setrlimit, RLIMIT_NOFILE, &lim), 0);
    expect("dup2_new_at_limit_ebadf", syscall2(SYS_dup2, fd, 64), -EBADF);
    expect("dup2_new_below_limit", syscall2(SYS_dup2, fd, 63), 63);
    a_close(63);
    expect("restore_limit", syscall2(SYS_setrlimit, RLIMIT_NOFILE, &old), 0);
    a_close(10);
    a_close(4);
    a_close(fd);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
