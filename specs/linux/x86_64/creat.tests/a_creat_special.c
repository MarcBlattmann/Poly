/* creat(2) on special files: /dev/null opens for writing (truncation does
 * nothing), a FIFO with a reader opens for writing and keeps its type, a
 * FIFO's data is not discarded by the truncation. */
#include "a_sys.h"

#define SYS_creat 85
#define SYS_mknodat 259
#define D "/tmp/a_creat_special"

static i64 cr(const char *p, i64 mode) { return syscall2(SYS_creat, p, mode); }

int test_main(void) {
    struct a_stat st;
    char buf[8];
    expect("setup_mkdir", a_mkdir(D, 0700), 0);

    i64 fd = cr("/dev/null", 0644);
    expect("devnull_fd", fd, 3);
    expect("devnull_write", a_write(fd, "abc", 3), 3);
    expect("devnull_getfl", syscall2(SYS_fcntl, fd, F_GETFL), O_WRONLY | O_LARGEFILE);
    expect("devnull_still_chr", syscall2(SYS_fstat, fd, &st) == 0 ? (i64)(st.st_mode & S_IFMT) : -1, S_IFCHR);
    a_close(fd);

    expect("mkfifo", syscall4(SYS_mknodat, AT_FDCWD, D "/p", S_IFIFO | 0600, 0), 0);
    i64 rd = a_open(D "/p", O_RDONLY | O_NONBLOCK, 0);
    expect("fifo_reader", rd, 3);
    fd = cr(D "/p", 0644);
    expect("fifo_creat_with_reader", fd, 4);
    expect("fifo_still_fifo", syscall2(SYS_fstat, fd, &st) == 0 ? (i64)st.st_mode : -1, S_IFIFO | 0600);
    expect("fifo_write", a_write(fd, "hi", 2), 2);
    i64 fd2 = cr(D "/p", 0644);
    expect("fifo_second_creat", fd2, 5);
    expect("fifo_data_kept", a_read(rd, buf, sizeof buf), 2);
    check("fifo_data", buf[0] == 'h' && buf[1] == 'i', buf[0]);
    a_close(fd2);
    a_close(fd);
    a_close(rd);

    expect("cleanup_p", a_unlink(D "/p"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
