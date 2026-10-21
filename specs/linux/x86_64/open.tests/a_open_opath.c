/* open(2) with O_PATH: a descriptor that only names a file. */
#include "a_sys.h"

#define SYS_open 2
#define D "/tmp/a_open_opath"

static i64 op(const char *p, i64 flags, i64 mode) { return syscall3(SYS_open, p, flags, mode); }

int test_main(void) {
    struct a_stat st;
    char c;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0600), 0);
    expect("setup_link", syscall2(SYS_symlink, "f", D "/l"), 0);

    i64 fd = op(D "/f", O_PATH, 0);
    expect("opath_file_fd", fd, 3);
    expect("opath_read_ebadf", a_read(fd, &c, 1), -EBADF);
    expect("opath_write_ebadf", a_write(fd, "y", 1), -EBADF);
    expect("opath_fstat", syscall2(SYS_fstat, fd, &st), 0);
    expect("opath_fstat_regular", st.st_mode, S_IFREG | 0600);
    expect("opath_getfl", syscall3(SYS_fcntl, fd, F_GETFL, 0), O_PATH);
    a_close(fd);

    /* The access mode bits are ignored with O_PATH. */
    fd = op(D "/f", O_PATH | O_RDWR, 0);
    expect("opath_rdwr_fd", fd, 3);
    expect("opath_rdwr_getfl", syscall3(SYS_fcntl, fd, F_GETFL, 0), O_PATH);
    expect("opath_rdwr_write_ebadf", a_write(fd, "y", 1), -EBADF);
    a_close(fd);

    /* O_PATH|O_NOFOLLOW on a link: the link itself. */
    fd = op(D "/l", O_PATH | O_NOFOLLOW, 0);
    expect("opath_nofollow_link_fd", fd, 3);
    expect("opath_nofollow_link_is_link",
           syscall2(SYS_fstat, fd, &st) == 0 ? (i64)(st.st_mode & S_IFMT) : -1, S_IFLNK);
    expect("opath_nofollow_getfl", syscall3(SYS_fcntl, fd, F_GETFL, 0), O_PATH | O_NOFOLLOW);
    a_close(fd);
    fd = op(D "/l", O_PATH, 0);
    expect("opath_follows_link", syscall2(SYS_fstat, fd, &st) == 0 ? (i64)(st.st_mode & S_IFMT) : -1, S_IFREG);
    a_close(fd);

    /* A directory: works as a dirfd and for fchdir. */
    fd = op(D, O_PATH | O_DIRECTORY, 0);
    expect("opath_dir_fd", fd, 3);
    expect("opath_dir_getdents_ebadf", syscall3(SYS_getdents64, fd, &st, sizeof st), -EBADF);
    i64 fd2 = syscall4(SYS_openat, fd, "f", O_RDONLY, 0);
    expect("opath_dir_as_dirfd", fd2, 4);
    a_close(fd2);
    expect("opath_dir_fchdir", syscall1(SYS_fchdir, fd), 0);
    fd2 = op("f", O_RDONLY, 0);
    expect("open_relative_after_fchdir", fd2, 4);
    a_close(fd2);
    syscall1(SYS_chdir, "/");
    a_close(fd);

    expect("opath_directory_on_file_enotdir", op(D "/f", O_PATH | O_DIRECTORY, 0), -ENOTDIR);
    expect("opath_missing_enoent", op(D "/nope", O_PATH, 0), -ENOENT);
    /* With O_PATH, flags other than O_DIRECTORY, O_NOFOLLOW, O_CLOEXEC are ignored. */
    fd = op(D "/new", O_PATH | O_CREAT, 0600);
    expect("opath_creat_ignored_enoent", fd, -ENOENT);
    fd = op(D "/f", O_PATH | O_TRUNC | O_APPEND, 0);
    expect("opath_trunc_ignored", fd, 3);
    syscall2(SYS_fstat, fd, &st);
    expect("opath_trunc_did_not_truncate", st.st_size, 1);
    a_close(fd);
    fd = op(D "/f", O_PATH | O_CLOEXEC, 0);
    expect("opath_cloexec", syscall3(SYS_fcntl, fd, F_GETFD, 0), FD_CLOEXEC);
    a_close(fd);
    fd = op(D, O_PATH | O_RDWR, 0);
    expect("opath_dir_rdwr_ok", fd, 3);
    a_close(fd);

    expect("cleanup_unlink_l", a_unlink(D "/l"), 0);
    expect("cleanup_unlink_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
