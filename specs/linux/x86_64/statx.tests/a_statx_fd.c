/* statx: on descriptors with AT_EMPTY_PATH (regular, O_PATH, directory),
 * relative to a directory descriptor, and timestamps set by utimensat. */
#include "a_sys.h"

#define D "/tmp/a_statx_fd"

static i64 statx(i64 dfd, const char *p, i64 flags, i64 mask, struct a_statx *sx) {
    return syscall5(SYS_statx, dfd, p, flags, mask, sx);
}

int test_main(void) {
    struct a_statx sx;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "abcd", 0600), 0);
    i64 fd = a_open(D "/f", O_RDWR, 0);
    expect("empty_path_fd", statx(fd, "", AT_EMPTY_PATH, STATX_BASIC_STATS, &sx), 0);
    expect("empty_path_size", sx.stx_size, 4);
    syscall4(SYS_pwrite64, fd, "efgh", 4, 4);
    expect("after_write", statx(fd, "", AT_EMPTY_PATH, STATX_SIZE, &sx), 0);
    expect("size_follows_writes", sx.stx_size, 8);
    struct a_timespec t[2] = {{1111111111, 5}, {1222222222, 999999999}};
    syscall4(SYS_utimensat, fd, 0, t, 0);
    expect("after_utimensat", statx(fd, "", AT_EMPTY_PATH, STATX_ATIME | STATX_MTIME, &sx), 0);
    check("atime_exact", sx.stx_atime.tv_sec == 1111111111 && sx.stx_atime.tv_nsec == 5, sx.stx_atime.tv_sec);
    check("mtime_exact", sx.stx_mtime.tv_sec == 1222222222 && sx.stx_mtime.tv_nsec == 999999999,
          sx.stx_mtime.tv_sec);
    a_close(fd);
    i64 p = a_open(D "/f", O_PATH, 0);
    expect("path_fd", statx(p, "", AT_EMPTY_PATH, STATX_BASIC_STATS, &sx), 0);
    expect("path_fd_mode", sx.stx_mode, S_IFREG | 0600);
    a_close(p);
    i64 dfd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("dir_fd_empty_path", statx(dfd, "", AT_EMPTY_PATH, STATX_BASIC_STATS, &sx), 0);
    expect("dir_fd_is_dir", sx.stx_mode & S_IFMT, S_IFDIR);
    expect("relative_to_dirfd", statx(dfd, "f", 0, STATX_BASIC_STATS, &sx), 0);
    expect("relative_size", sx.stx_size, 8);
    expect("relative_missing_enoent", statx(dfd, "g", 0, STATX_BASIC_STATS, &sx), -ENOENT);
    a_close(dfd);
    expect("closed_fd_empty_path_ebadf", statx(dfd, "", AT_EMPTY_PATH, STATX_BASIC_STATS, &sx), -EBADF);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
