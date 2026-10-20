/* dup3: O_CLOEXEC is the only flag; the same descriptor twice is EINVAL
 * (unlike dup2). */
#include "a_sys.h"

#define F "/tmp/a_dup3_flags"

int test_main(void) {
    expect("setup_file", a_make_file(F, "abc", 0644), 0);
    i64 fd = a_open(F, O_RDONLY, 0);
    expect("dup3_cloexec", syscall3(SYS_dup3, fd, 10, O_CLOEXEC), 10);
    expect("target_has_cloexec", syscall3(SYS_fcntl, 10, F_GETFD, 0), FD_CLOEXEC);
    expect("dup3_no_flags", syscall3(SYS_dup3, fd, 11, 0), 11);
    expect("target_no_cloexec", syscall3(SYS_fcntl, 11, F_GETFD, 0), 0);
    /* Replacing a close-on-exec target without the flag clears it. */
    expect("dup3_over_cloexec_target", syscall3(SYS_dup3, fd, 10, 0), 10);
    expect("replaced_target_no_cloexec", syscall3(SYS_fcntl, 10, F_GETFD, 0), 0);
    expect("dup3_same_einval", syscall3(SYS_dup3, fd, fd, 0), -EINVAL);
    expect("dup3_same_cloexec_einval", syscall3(SYS_dup3, fd, fd, O_CLOEXEC), -EINVAL);
    expect("dup3_bad_flag_einval", syscall3(SYS_dup3, fd, 12, O_NONBLOCK), -EINVAL);
    expect("dup3_bad_flag_left_target_free", syscall3(SYS_fcntl, 12, F_GETFD, 0), -EBADF);
    expect("dup3_unknown_flag_einval", syscall3(SYS_dup3, fd, 12, 1), -EINVAL);
    expect("dup3_bad_old_ebadf", syscall3(SYS_dup3, 30, 12, 0), -EBADF);
    expect("dup3_negative_new_ebadf", syscall3(SYS_dup3, fd, -1, 0), -EBADF);
    expect("dup3_closed_same_einval", syscall3(SYS_dup3, 31, 31, 0), -EINVAL);
    a_close(10);
    a_close(11);
    a_close(fd);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
