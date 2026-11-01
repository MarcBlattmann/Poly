/* pwritev2(2) is pwritev with flags. Offset -1 means the file position,
 * which is used and advanced (like writev; with O_APPEND the write goes
 * to the end and the position follows); other negative offsets are
 * EINVAL; with -1 a pipe can be written, an explicit offset is ESPIPE. */
#include "a_sys.h"

#define D "/tmp/lb_pwritev2_offset"
#define SYS_pwritev2 328

static i64 pwritev2_(i64 fd, struct a_iovec *iov, i64 cnt, i64 lo, i64 hi, i64 flags) {
    return syscall6(SYS_pwritev2, fd, (i64)iov, cnt, lo, hi, flags);
}

int test_main(void) {
    char back[32];
    struct a_iovec v[2] = {{"ab", 2}, {"cd", 2}};
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    i64 fd = a_open(D "/f", O_RDWR | O_CREAT, 0644);
    expect("minus1_writes_at_pos", pwritev2_(fd, v, 2, -1, 0, 0), 4);
    expect("pos_4", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 4);
    expect("minus1_again", pwritev2_(fd, v, 1, -1, 7, 0), 2);
    expect("pos_6", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 6);
    expect("explicit", pwritev2_(fd, v, 1, 1, 0, 0), 2);
    expect("explicit_keeps_pos", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 6);
    expect("read_back", syscall4(SYS_pread64, fd, back, 32, 0), 6);
    expect("contents", mem_eq(back, "aabdab", 6), 1);
    expect("minus2_einval", pwritev2_(fd, v, 1, -2, 0, 0), -EINVAL);
    /* O_APPEND with -1: at the end, the position moves to the end */
    i64 ap = a_open(D "/f", O_WRONLY | O_APPEND, 0);
    expect("append_minus1", pwritev2_(ap, v, 1, -1, 0, 0), 2);
    expect("append_pos_8", syscall3(SYS_lseek, ap, 0, SEEK_CUR), 8);
    /* O_APPEND with an explicit offset: at the end, the position stays */
    expect("append_explicit", pwritev2_(ap, v, 1, 0, 0, 0), 2);
    expect("append_explicit_pos_8", syscall3(SYS_lseek, ap, 0, SEEK_CUR), 8);
    expect("append_read_back", syscall4(SYS_pread64, fd, back, 32, 0), 10);
    expect("append_contents", mem_eq(back, "aabdababab", 10), 1);
    /* pipes */
    i32 pp[2];
    syscall2(SYS_pipe2, pp, 0);
    expect("pipe_explicit_espipe", pwritev2_(pp[1], v, 1, 0, 0, 0), -ESPIPE);
    expect("pipe_minus1", pwritev2_(pp[1], v, 2, -1, 0, 0), 4);
    expect("pipe_read", a_read(pp[0], back, 32) == 4 && mem_eq(back, "abcd", 4), 1);
    /* errors shared with pwritev */
    i64 ro = a_open(D "/f", O_RDONLY, 0);
    expect("rdonly", pwritev2_(ro, v, 1, -1, 0, 0), -EBADF);
    expect("count_1025", pwritev2_(fd, v, 1025, -1, 0, 0), -EINVAL);

    a_close(ro);
    a_close(ap);
    a_close(fd);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
