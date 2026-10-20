/* fcntl: F_GETFL and F_SETFL; F_SETFL changes O_APPEND and O_NONBLOCK and
 * ignores the access mode and creation flags. */
#include "a_sys.h"

#define F "/tmp/a_fcntl_status"

static i64 fcntl(i64 fd, i64 cmd, i64 arg) { return syscall3(SYS_fcntl, fd, cmd, arg); }

int test_main(void) {
    expect("setup_file", a_make_file(F, "abc", 0644), 0);
    i64 fd = a_open(F, O_RDONLY, 0);
    expect("getfl_rdonly", fcntl(fd, F_GETFL, 0), O_RDONLY | O_LARGEFILE);
    expect("setfl_append_nonblock", fcntl(fd, F_SETFL, O_APPEND | O_NONBLOCK), 0);
    expect("getfl_after_set", fcntl(fd, F_GETFL, 0), O_RDONLY | O_APPEND | O_NONBLOCK | O_LARGEFILE);
    expect("setfl_access_mode_ignored", fcntl(fd, F_SETFL, O_RDWR), 0);
    expect("still_rdonly_flags_cleared", fcntl(fd, F_GETFL, 0), O_RDONLY | O_LARGEFILE);
    expect("still_cannot_write", a_write(fd, "x", 1), -EBADF);
    expect("setfl_creation_flags_ignored", fcntl(fd, F_SETFL, O_CREAT | O_EXCL | O_TRUNC | O_NOCTTY), 0);
    expect("creation_flags_not_set", fcntl(fd, F_GETFL, 0), O_RDONLY | O_LARGEFILE);
    struct a_stat st;
    syscall2(SYS_fstat, fd, &st);
    expect("setfl_trunc_did_not_truncate", st.st_size, 3);
    expect("setfl_cloexec_ignored", fcntl(fd, F_SETFL, O_CLOEXEC), 0);
    expect("cloexec_not_fd_flag", fcntl(fd, F_GETFD, 0), 0);
    /* On x86-64, F_SETFL always keeps O_LARGEFILE. */
    expect("setfl_0", fcntl(fd, F_SETFL, 0), 0);
    expect("largefile_kept", fcntl(fd, F_GETFL, 0), O_RDONLY | O_LARGEFILE);
    a_close(fd);
    fd = a_open(F, O_WRONLY | O_APPEND, 0);
    expect("getfl_wronly_append", fcntl(fd, F_GETFL, 0), O_WRONLY | O_APPEND | O_LARGEFILE);
    expect("clear_append", fcntl(fd, F_SETFL, 0), 0);
    expect("getfl_wronly", fcntl(fd, F_GETFL, 0), O_WRONLY | O_LARGEFILE);
    a_close(fd);
    fd = a_open(F, O_PATH, 0);
    expect("path_getfl", fcntl(fd, F_GETFL, 0), O_PATH);
    expect("path_setfl_ebadf", fcntl(fd, F_SETFL, O_APPEND), -EBADF);
    a_close(fd);
    expect("closed_getfl_ebadf", fcntl(fd, F_GETFL, 0), -EBADF);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
