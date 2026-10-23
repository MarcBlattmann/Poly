/* fadvise64: both ends of a pipe and an opened FIFO give ESPIPE, for every
 * advice, and ESPIPE is reported before a bad advice or a negative
 * length. */
#include "a_sys.h"

#define SYS_fadvise64 221
#define SYS_mknodat 259
#define D "/tmp/fadvise64_pipe"

static i64 fadv(i64 fd, i64 off, i64 len, i64 advice) { return syscall4(SYS_fadvise64, fd, off, len, advice); }

int test_main(void) {
    int p[2] = {-1, -1};
    expect("pipe2", syscall2(SYS_pipe2, p, 0), 0);
    expect("read_end_normal", fadv(p[0], 0, 0, 0), -ESPIPE);
    expect("read_end_willneed", fadv(p[0], 0, 4096, 3), -ESPIPE);
    expect("read_end_dontneed", fadv(p[0], 0, 0, 4), -ESPIPE);
    expect("write_end_sequential", fadv(p[1], 0, 0, 2), -ESPIPE);
    expect("write_end_noreuse", fadv(p[1], 0, 0, 5), -ESPIPE);
    expect("bad_advice_espipe", fadv(p[0], 0, 0, 99), -ESPIPE);
    expect("negative_len_espipe", fadv(p[1], 0, -1, 0), -ESPIPE);
    expect("negative_offset_espipe", fadv(p[1], -4096, 0, 0), -ESPIPE);
    expect("write_still_works", a_write(p[1], "x", 1), 1);
    a_close(p[0]);
    a_close(p[1]);
    a_mkdir(D, 0755);
    expect("mkfifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", S_IFIFO | 0644, 0), 0);
    i64 f = a_open(D "/fifo", O_RDWR, 0);
    check("open_fifo", f >= 0, f);
    expect("fifo_normal", fadv(f, 0, 0, 0), -ESPIPE);
    expect("fifo_random", fadv(f, 0, 0, 1), -ESPIPE);
    expect("fifo_bad_advice", fadv(f, 0, 0, 6), -ESPIPE);
    a_close(f);
    i64 r = a_open(D "/fifo", O_RDONLY | O_NONBLOCK, 0);
    check("open_fifo_nonblock", r >= 0, r);
    expect("fifo_nonblock_read_end", fadv(r, 0, 0, 3), -ESPIPE);
    a_close(r);
    a_unlink(D "/fifo");
    a_rmdir(D);
    return finish();
}
