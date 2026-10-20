/* openat: which flags F_GETFL reports, O_CLOEXEC, O_TMPFILE and the
 * access mode 3. */
#include "a_sys.h"

#define D "/tmp/a_openat_status"

static i64 getfl(i64 fd) { return syscall3(SYS_fcntl, fd, F_GETFL, 0); }
static i64 getfd(i64 fd) { return syscall3(SYS_fcntl, fd, F_GETFD, 0); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0700), 0);

    i64 fd = a_open(D "/f", O_RDWR | O_CREAT | O_TRUNC | O_EXCL | O_APPEND | O_NONBLOCK, 0600);
    expect("open_many_flags", fd, 3);
    /* x86-64 opens are always large-file opens; creation flags are not kept. */
    expect("getfl_keeps_status_flags_only", getfl(fd), O_RDWR | O_APPEND | O_NONBLOCK | O_LARGEFILE);
    expect("no_cloexec_by_default", getfd(fd), 0);
    a_close(fd);

    fd = a_open(D "/f", O_RDONLY | O_CLOEXEC, 0);
    expect("cloexec_open", fd, 3);
    expect("cloexec_sets_fd_flag", getfd(fd), FD_CLOEXEC);
    expect("cloexec_not_in_getfl", getfl(fd), O_RDONLY | O_LARGEFILE);
    a_close(fd);

    fd = a_open(D "/f", O_WRONLY | O_NOCTTY | O_NOFOLLOW, 0);
    /* O_NOFOLLOW is reported back, O_NOCTTY is not. */
    expect("wronly_getfl_keeps_nofollow", getfl(fd), O_WRONLY | O_NOFOLLOW | O_LARGEFILE);
    a_close(fd);

    fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("dir_getfl_keeps_directory", getfl(fd), O_RDONLY | O_DIRECTORY | O_LARGEFILE);
    a_close(fd);
    /* O_PATH keeps only O_PATH, O_DIRECTORY and O_NOFOLLOW (no O_LARGEFILE). */
    fd = a_open(D, O_PATH | O_DIRECTORY | O_RDWR | O_APPEND, 0);
    expect("path_getfl", getfl(fd), O_PATH | O_DIRECTORY);
    a_close(fd);

    /* Unknown flag bits are ignored by openat. */
    fd = a_open(D "/f", O_RDONLY | 0x40000000, 0);
    expect("unknown_flag_ignored", fd, 3);
    a_close(fd);

    /* Access mode 3: opens, but neither reads nor writes. */
    fd = a_open(D "/f", 3, 0);
    expect("accmode_3_opens", fd, 3);
    char c;
    expect("accmode_3_read_ebadf", a_read(fd, &c, 1), -EBADF);
    expect("accmode_3_write_ebadf", a_write(fd, "x", 1), -EBADF);
    a_close(fd);

    /* O_TMPFILE: an unnamed regular file in the directory. */
    fd = a_open(D, O_TMPFILE | O_RDWR, 0600);
    expect("tmpfile_opens", fd, 3);
    struct a_stat st;
    syscall2(SYS_fstat, fd, &st);
    expect("tmpfile_is_regular", st.st_mode & S_IFMT, S_IFREG);
    expect("tmpfile_has_no_links", st.st_nlink, 0);
    expect("tmpfile_write", a_write(fd, "abc", 3), 3);
    a_close(fd);
    expect("tmpfile_without_write_access_einval", a_open(D, O_TMPFILE | O_RDONLY, 0600), -EINVAL);
    expect("tmpfile_on_file_enotdir", a_open(D "/f", O_TMPFILE | O_RDWR, 0600), -ENOTDIR);

    expect("cleanup_unlink", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
