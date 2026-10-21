/* open(2) with O_TMPFILE: an unnamed file in a directory, its mode and
 * link count, giving it a name, and the errors. */
#include "a_sys.h"

#define SYS_open 2
#define D "/tmp/a_open_tmpfile"

static i64 op(const char *p, i64 flags, i64 mode) { return syscall3(SYS_open, p, flags, mode); }

int test_main(void) {
    struct a_stat st;
    char buf[64];
    static char dents[1024] __attribute__((aligned(8)));
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0600), 0);

    i64 fd = op(D, O_RDWR | O_TMPFILE, 0640);
    expect("tmpfile_rdwr_fd", fd, 3);
    expect("tmpfile_fstat", syscall2(SYS_fstat, fd, &st), 0);
    expect("tmpfile_is_regular_mode", st.st_mode, S_IFREG | 0640);
    expect("tmpfile_nlink_0", st.st_nlink, 0);
    expect("tmpfile_size_0", st.st_size, 0);
    expect("tmpfile_getfl", syscall3(SYS_fcntl, fd, F_GETFL, 0), O_RDWR | O_TMPFILE | O_LARGEFILE);
    expect("tmpfile_write", a_write(fd, "hello", 5), 5);
    expect("tmpfile_seek", syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);
    expect("tmpfile_read", a_read(fd, buf, sizeof buf), 5);
    /* The directory has no new entry. */
    i64 dfd = op(D, O_RDONLY | O_DIRECTORY, 0);
    i64 n = syscall3(SYS_getdents64, dfd, dents, sizeof dents), count = 0;
    for (i64 off = 0; off < n; off += ((struct a_dirent64 *)(dents + off))->d_reclen) count++;
    expect("dir_still_three_entries", count, 3);
    a_close(dfd);

    /* linkat(fd, "", dir, name, AT_EMPTY_PATH) gives it a name. */
    expect("linkat_names_it", syscall5(SYS_linkat, fd, "", AT_FDCWD, D "/named", AT_EMPTY_PATH), 0);
    expect("named_stat", a_stat_path(D "/named", &st), 0);
    expect("named_size", st.st_size, 5);
    expect("named_nlink_1", st.st_nlink, 1);
    a_close(fd);

    fd = op(D, O_WRONLY | O_TMPFILE, 0777);
    expect("tmpfile_wronly_fd", fd, 3);
    syscall2(SYS_fstat, fd, &st);
    expect("tmpfile_umask_applied", st.st_mode, S_IFREG | 0755);
    a_close(fd);

    /* O_TMPFILE|O_EXCL: the file can never be linked. */
    fd = op(D, O_RDWR | O_TMPFILE | O_EXCL, 0600);
    expect("tmpfile_excl_fd", fd, 3);
    expect("tmpfile_excl_linkat_enoent", syscall5(SYS_linkat, fd, "", AT_FDCWD, D "/never", AT_EMPTY_PATH), -ENOENT);
    a_close(fd);

    expect("tmpfile_rdonly_einval", op(D, O_RDONLY | O_TMPFILE, 0600), -EINVAL);
    expect("tmpfile_on_file_enotdir", op(D "/f", O_RDWR | O_TMPFILE, 0600), -ENOTDIR);
    expect("tmpfile_on_missing_enoent", op(D "/nope", O_RDWR | O_TMPFILE, 0600), -ENOENT);
    expect("tmpfile_with_creat_einval", op(D, O_RDWR | O_TMPFILE | O_CREAT, 0600), -EINVAL);
    /* The bit without O_DIRECTORY is not O_TMPFILE. */
    expect("tmpfile_bit_alone_on_dir_einval", op(D, O_RDWR | 020000000, 0600), -EINVAL);

    expect("cleanup_unlink_named", a_unlink(D "/named"), 0);
    expect("cleanup_unlink_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
