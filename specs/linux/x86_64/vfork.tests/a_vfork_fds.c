/* vfork: the child shares the parent's memory but has its own copy of the
 * descriptor table: descriptors it closes or opens are not the parent's,
 * while the open files (and offsets) are shared. */
#include "a_sys.h"

#define F "/tmp/a_vfork_fds"

static volatile i64 closed, opened, read_n;

int test_main(void) {
    a_block_sigchld();
    a_make_file(F, "abcdef", 0644);
    i64 fd = a_open(F, O_RDONLY, 0);
    i64 pid = syscall0(SYS_vfork);
    if (pid == 0) {
        char buf[2];
        read_n = a_read(fd, buf, 2);
        closed = a_close(fd);
        opened = a_open("/", O_RDONLY | O_DIRECTORY, 0);
        syscall1(SYS_exit_group, 0);
    }
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    check("vfork", pid > 0, pid);
    expect("child_read", read_n, 2);
    expect("child_close_ok", closed, 0);
    expect("parent_fd_still_open", syscall3(SYS_fcntl, fd, F_GETFD, 0), 0);
    expect("offset_shared", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 2);
    expect("child_open_got_freed_number", opened, fd);
    expect("child_fd_not_in_parent_table", syscall3(SYS_fcntl, fd + 1, F_GETFD, 0), -EBADF);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
