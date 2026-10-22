/* fdatasync changes nothing visible: file contents, size, offset, mode
 * and the three timestamps are the same before and after; the flushed data
 * reads back through another descriptor. */
#include "a_sys.h"

#define F "/tmp/c_fdatasync_no_change"

int test_main(void) {
    struct a_stat a, b;
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0640);
    expect("write", a_write(fd, "hello world", 11), 11);
    expect("seek", syscall3(SYS_lseek, fd, 3, SEEK_SET), 3);
    expect("fstat_before", syscall2(SYS_fstat, fd, &a), 0);
    struct { i64 s, ns; } delay = {0, 20000000};
    expect("sleep", syscall2(SYS_nanosleep, &delay, 0), 0);
    expect("fdatasync", syscall1(SYS_fdatasync, fd), 0);
    expect("fstat_after", syscall2(SYS_fstat, fd, &b), 0);
    expect("size", b.st_size, 11);
    check("mode", b.st_mode == a.st_mode, b.st_mode);
    check("nlink", b.st_nlink == a.st_nlink, b.st_nlink);
    check("mtime", b.st_mtime_sec == a.st_mtime_sec && b.st_mtime_nsec == a.st_mtime_nsec, 0);
    check("ctime", b.st_ctime_sec == a.st_ctime_sec && b.st_ctime_nsec == a.st_ctime_nsec, 0);
    check("atime", b.st_atime_sec == a.st_atime_sec && b.st_atime_nsec == a.st_atime_nsec, 0);
    expect("offset", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 3);
    char buf[16];
    expect("read_on_from_offset", a_read(fd, buf, 4), 4);
    check("read_bytes", mem_eq(buf, "lo w", 4), 0);
    i64 other = a_open(F, O_RDONLY, 0);
    expect("other_read", a_read(other, buf, sizeof buf), 11);
    check("other_bytes", mem_eq(buf, "hello world", 11), 0);
    a_close(other);
    expect("twice", syscall1(SYS_fdatasync, fd), 0);
    expect("thrice", syscall1(SYS_fdatasync, fd), 0);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
