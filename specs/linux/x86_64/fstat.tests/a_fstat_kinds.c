/* fstat: directories, pipes, /dev/null and O_PATH descriptors; bad
 * descriptors and buffers. */
#include "a_sys.h"

#define D "/tmp/a_fstat_kinds"

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0750), 0);
    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("dir_fstat", syscall2(SYS_fstat, fd, &st), 0);
    expect("dir_mode", st.st_mode, S_IFDIR | 0750);
    expect("empty_dir_nlink_2", st.st_nlink, 2);
    a_close(fd);

    i32 p[2];
    syscall2(SYS_pipe2, p, 0);
    expect("pipe_fstat", syscall2(SYS_fstat, p[0], &st), 0);
    expect("pipe_type_fifo", st.st_mode & S_IFMT, S_IFIFO);
    expect("pipe_write_end_fifo", syscall2(SYS_fstat, p[1], &st) == 0 ? (i64)(st.st_mode & S_IFMT) : -1, S_IFIFO);
    a_close(p[0]);
    a_close(p[1]);

    fd = a_open("/dev/null", O_RDWR, 0);
    expect("devnull_open", fd, 3);
    expect("devnull_fstat", syscall2(SYS_fstat, fd, &st), 0);
    expect("devnull_is_char_device", st.st_mode & S_IFMT, S_IFCHR);
    expect("devnull_rdev_1_3", st.st_rdev, (1 << 8) | 3);
    expect("devnull_mode_0666", st.st_mode & 07777, 0666);
    expect("devnull_size_0", st.st_size, 0);
    a_close(fd);

    fd = a_open(D, O_PATH, 0);
    expect("path_fd_fstat", syscall2(SYS_fstat, fd, &st), 0);
    expect("path_fd_dir_mode", st.st_mode, S_IFDIR | 0750);
    expect("bad_buffer_efault", syscall2(SYS_fstat, fd, (void *)16), -EFAULT);
    a_close(fd);
    expect("closed_ebadf", syscall2(SYS_fstat, fd, &st), -EBADF);
    expect("negative_ebadf", syscall2(SYS_fstat, -1, &st), -EBADF);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
