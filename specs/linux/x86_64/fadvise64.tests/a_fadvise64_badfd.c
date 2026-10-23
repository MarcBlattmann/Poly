/* fadvise64: a descriptor that is not open, AT_FDCWD, or an O_PATH
 * descriptor (of a file or of a FIFO) is EBADF, and EBADF is reported
 * before a bad advice or a negative length. */
#include "a_sys.h"

#define SYS_fadvise64 221
#define SYS_mknodat 259
#define D "/tmp/fadvise64_badfd"

static i64 fadv(i64 fd, i64 off, i64 len, i64 advice) { return syscall4(SYS_fadvise64, fd, off, len, advice); }

int test_main(void) {
    a_mkdir(D, 0755);
    expect("make_file", a_make_file(D "/f", "data", 0644), 0);
    expect("minus_one", fadv(-1, 0, 0, 0), -EBADF);
    expect("at_fdcwd", fadv(AT_FDCWD, 0, 0, 0), -EBADF);
    expect("never_opened", fadv(900, 0, 0, 3), -EBADF);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    check("open", fd >= 0, fd);
    expect("open_ok", fadv(fd, 0, 0, 0), 0);
    a_close(fd);
    expect("closed", fadv(fd, 0, 0, 0), -EBADF);
    expect("closed_bad_advice", fadv(fd, 0, 0, 99), -EBADF);
    expect("closed_negative_len", fadv(fd, 0, -1, 0), -EBADF);
    expect("minus_one_bad_advice", fadv(-1, 0, -1, 99), -EBADF);
    i64 p = a_open(D "/f", O_PATH, 0);
    check("open_path", p >= 0, p);
    expect("o_path_file", fadv(p, 0, 0, 0), -EBADF);
    expect("o_path_file_dontneed", fadv(p, 0, 0, 4), -EBADF);
    expect("o_path_bad_advice", fadv(p, 0, 0, 99), -EBADF);
    a_close(p);
    expect("mkfifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", S_IFIFO | 0644, 0), 0);
    i64 q = a_open(D "/fifo", O_PATH, 0);
    check("open_path_fifo", q >= 0, q);
    expect("o_path_fifo", fadv(q, 0, 0, 0), -EBADF);
    a_close(q);
    a_unlink(D "/fifo");
    a_unlink(D "/f");
    a_rmdir(D);
    return finish();
}
