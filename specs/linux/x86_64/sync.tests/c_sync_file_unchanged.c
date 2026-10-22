/* sync() leaves an open file as it was: contents, size, the three
 * timestamps, link count and the descriptor's offset do not change. */
#include "a_sys.h"

#define SYS_sync 162
#define F "/tmp/c_sync_file_unchanged"

int test_main(void) {
    struct a_stat a, b;
    char buf[16];
    a_unlink(F);
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    expect("write", a_write(fd, "hello, sync", 11), 11);
    expect("fstat_before", syscall2(SYS_fstat, fd, &a), 0);
    expect("sync", syscall0(SYS_sync), 0);
    expect("fstat_after", syscall2(SYS_fstat, fd, &b), 0);
    expect("size", b.st_size, 11);
    check("mtime", a.st_mtime_sec == b.st_mtime_sec && a.st_mtime_nsec == b.st_mtime_nsec, b.st_mtime_nsec);
    check("ctime", a.st_ctime_sec == b.st_ctime_sec && a.st_ctime_nsec == b.st_ctime_nsec, b.st_ctime_nsec);
    check("atime", a.st_atime_sec == b.st_atime_sec && a.st_atime_nsec == b.st_atime_nsec, b.st_atime_nsec);
    expect("nlink", b.st_nlink, 1);
    check("ino", a.st_ino == b.st_ino, b.st_ino);
    expect("offset", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 11);
    expect("pread", syscall4(SYS_pread64, fd, buf, sizeof buf, 0), 11);
    check("contents", buf[0] == 'h' && buf[7] == 's' && buf[10] == 'c', buf[0]);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
