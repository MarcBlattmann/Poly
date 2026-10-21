/* open(2) on directories: read-only works, writing and O_CREAT do not,
 * O_DIRECTORY, and what a directory descriptor can do. */
#include "a_sys.h"

#define SYS_open 2
#define D "/tmp/a_open_dirs"

static i64 op(const char *p, i64 flags, i64 mode) { return syscall3(SYS_open, p, flags, mode); }

int test_main(void) {
    char buf[512];
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0600), 0);

    i64 fd = op(D, O_RDONLY, 0);
    expect("dir_rdonly_opens", fd, 3);
    expect("dir_read_eisdir", a_read(fd, buf, 1), -EISDIR);
    expect("dir_fstat_is_dir", syscall2(SYS_fstat, fd, &st) == 0 ? (i64)(st.st_mode & S_IFMT) : -1, S_IFDIR);
    expect("dir_getdents_has_entries", syscall3(SYS_getdents64, fd, buf, sizeof buf) > 0, 1);
    expect("dir_getfl", syscall3(SYS_fcntl, fd, F_GETFL, 0), O_RDONLY | O_LARGEFILE);
    a_close(fd);

    fd = op(D, O_RDONLY | O_DIRECTORY, 0);
    expect("dir_o_directory_opens", fd, 3);
    expect("dir_o_directory_getfl", syscall3(SYS_fcntl, fd, F_GETFL, 0), O_RDONLY | O_DIRECTORY | O_LARGEFILE);
    a_close(fd);
    fd = op(D "/", O_RDONLY, 0);
    expect("dir_trailing_slash_opens", fd, 3);
    a_close(fd);
    fd = op(D "/.", O_RDONLY, 0);
    expect("dir_dot_opens", fd, 3);
    a_close(fd);
    fd = op("/", O_RDONLY | O_DIRECTORY, 0);
    expect("root_opens", fd, 3);
    a_close(fd);

    expect("dir_wronly_eisdir", op(D, O_WRONLY, 0), -EISDIR);
    expect("dir_rdwr_eisdir", op(D, O_RDWR, 0), -EISDIR);
    expect("dir_creat_rdonly_eisdir", op(D, O_RDONLY | O_CREAT, 0600), -EISDIR);
    expect("dir_creat_excl_eexist", op(D, O_RDONLY | O_CREAT | O_EXCL, 0600), -EEXIST);
    expect("dir_trunc_rdonly_eisdir", op(D, O_RDONLY | O_TRUNC, 0), -EISDIR);
    expect("dir_wronly_o_directory_eisdir", op(D, O_WRONLY | O_DIRECTORY, 0), -EISDIR);
    fd = op(D, 3, 0);
    expect("dir_accmode_3", fd, -EISDIR);
    if (fd >= 0) a_close(fd);

    expect("o_directory_on_file_enotdir", op(D "/f", O_RDONLY | O_DIRECTORY, 0), -ENOTDIR);
    expect("o_directory_on_missing_enoent", op(D "/nope", O_RDONLY | O_DIRECTORY, 0), -ENOENT);
    expect("o_directory_on_dev_null_enotdir", op("/dev/null", O_RDONLY | O_DIRECTORY, 0), -ENOTDIR);
    expect("creat_directory_new_einval", op(D "/new", O_RDWR | O_CREAT | O_DIRECTORY, 0600), -EINVAL);
    expect("creat_directory_new_not_created", a_stat_path(D "/new", &st), -ENOENT);
    expect("creat_directory_rdonly_new_einval", op(D "/new", O_RDONLY | O_CREAT | O_DIRECTORY, 0600), -EINVAL);
    expect("creat_directory_existing_dir_einval", op(D, O_RDONLY | O_CREAT | O_DIRECTORY, 0600), -EINVAL);
    expect("creat_directory_existing_file_einval", op(D "/f", O_RDONLY | O_CREAT | O_DIRECTORY, 0600), -EINVAL);

    /* A directory descriptor keeps working after the directory is gone. */
    expect("setup_sub", a_mkdir(D "/sub", 0700), 0);
    fd = op(D "/sub", O_RDONLY | O_DIRECTORY, 0);
    expect("sub_opens", fd, 3);
    expect("sub_rmdir", a_rmdir(D "/sub"), 0);
    expect("removed_dir_fstat", syscall2(SYS_fstat, fd, &st), 0);
    expect("removed_dir_nlink_0", st.st_nlink, 0);
    a_close(fd);

    expect("cleanup_unlink", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
