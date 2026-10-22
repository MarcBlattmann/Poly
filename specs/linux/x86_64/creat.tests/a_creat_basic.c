/* creat(2) (x86-64 syscall 85) on a new name: the descriptor number, the
 * access mode and flags of the open file, the new file's mode, size, link
 * count and owner, and reading versus writing through the descriptor. */
#include "a_sys.h"

#define SYS_creat 85
#define D "/tmp/a_creat_basic"

static i64 cr(const char *p, i64 mode) { return syscall2(SYS_creat, p, mode); }

int test_main(void) {
    struct a_stat st;
    char buf[8];
    expect("setup_mkdir", a_mkdir(D, 0700), 0);

    i64 fd = cr(D "/a", 0666);
    expect("new_fd_3", fd, 3);
    expect("new_fstat", syscall2(SYS_fstat, fd, &st), 0);
    expect("new_is_regular_umask_022", st.st_mode, S_IFREG | 0644);
    expect("new_size_0", st.st_size, 0);
    expect("new_nlink_1", st.st_nlink, 1);
    expect("new_owner_uid", st.st_uid, syscall0(SYS_geteuid));
    expect("new_owner_gid", st.st_gid, syscall0(SYS_getegid));
    expect("getfl_wronly_largefile", syscall2(SYS_fcntl, fd, F_GETFL), O_WRONLY | O_LARGEFILE);
    expect("getfd_no_cloexec", syscall2(SYS_fcntl, fd, F_GETFD), 0);
    expect("offset_starts_0", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 0);
    expect("write_ok", a_write(fd, "hello", 5), 5);
    expect("offset_after_write", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 5);
    expect("rewind", syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);
    expect("read_ebadf", a_read(fd, buf, sizeof buf), -EBADF);

    /* The lowest free number is handed out, and reused after close. */
    i64 fd2 = cr(D "/b", 0600);
    expect("second_fd_4", fd2, 4);
    a_close(fd);
    i64 fd3 = cr(D "/c", 0600);
    expect("reuses_fd_3", fd3, 3);
    a_close(fd3);
    a_close(fd2);

    /* The data written is there for a later reader. */
    fd = a_open(D "/a", O_RDONLY, 0);
    expect("reopen_rdonly", fd, 3);
    expect("reread_5", a_read(fd, buf, sizeof buf), 5);
    check("reread_data", buf[0] == 'h' && buf[4] == 'o', buf[0]);
    a_close(fd);

    /* A name in the directory listing: stat by path agrees. */
    expect("stat_by_path", a_stat_path(D "/b", &st), 0);
    expect("b_mode_0600", st.st_mode, S_IFREG | 0600);

    expect("cleanup_a", a_unlink(D "/a"), 0);
    expect("cleanup_b", a_unlink(D "/b"), 0);
    expect("cleanup_c", a_unlink(D "/c"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
