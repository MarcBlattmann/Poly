/* unlink: a file stays alive while a descriptor is open on it. Reads and
 * writes through the descriptor keep working, fstat shows nlink 0, and a
 * new file under the same name is a different file. */
#include "a_sys.h"

#define D "/tmp/c_unlink_open_fd"

static i64 u(const char *p) { return syscall1(SYS_unlink, p); }

int test_main(void) {
    struct a_stat st, st2;
    char buf[16];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    i64 fd = a_open(D "/f", O_RDWR | O_CREAT | O_EXCL, 0644);
    expect("open_ok", fd >= 0, 1);
    expect("write_before", a_write(fd, "abcdef", 6), 6);
    expect("fstat_before", syscall2(SYS_fstat, fd, &st), 0);
    expect("nlink_1_before", st.st_nlink, 1);
    u64 ino = st.st_ino;
    expect("unlink_open_file", u(D "/f"), 0);
    expect("fstat_after", syscall2(SYS_fstat, fd, &st), 0);
    expect("nlink_0_after", st.st_nlink, 0);
    expect("same_inode", st.st_ino == ino, 1);
    expect("size_kept", st.st_size, 6);
    expect("still_regular", st.st_mode & S_IFMT, S_IFREG);
    expect("write_after", a_write(fd, "gh", 2), 2);
    expect("pread_after", syscall4(SYS_pread64, fd, buf, sizeof buf, 0), 8);
    expect("data_intact", mem_eq(buf, "abcdefgh", 8), 1);
    expect("ftruncate_after", syscall2(SYS_ftruncate, fd, 3), 0);
    expect("fstat_truncated", syscall2(SYS_fstat, fd, &st), 0);
    expect("size_3", st.st_size, 3);
    expect("name_gone", a_stat_path(D "/f", &st2), -ENOENT);
    /* The same name now makes a new, separate file. */
    i64 fd2 = a_open(D "/f", O_RDWR | O_CREAT | O_EXCL, 0644);
    expect("new_file_same_name", fd2 >= 0, 1);
    expect("new_fstat", syscall2(SYS_fstat, fd2, &st2), 0);
    expect("new_inode_differs", st2.st_ino != ino, 1);
    expect("new_empty", st2.st_size, 0);
    expect("new_nlink_1", st2.st_nlink, 1);
    expect("old_still_3", syscall2(SYS_fstat, fd, &st), 0);
    expect("old_size_3", st.st_size, 3);
    /* A second descriptor keeps it alive after the first is closed. */
    i64 dupfd = syscall1(SYS_dup, fd);
    expect("close_first", a_close(fd), 0);
    expect("pread_dup", syscall4(SYS_pread64, dupfd, buf, sizeof buf, 0), 3);
    expect("dup_data", mem_eq(buf, "abc", 3), 1);
    a_close(dupfd);
    a_close(fd2);
    expect("cleanup_f", u(D "/f"), 0);
    expect("cleanup_dir", syscall1(SYS_rmdir, D), 0);
    return finish();
}
