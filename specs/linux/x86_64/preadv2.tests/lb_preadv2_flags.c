/* preadv2(2) flags: RWF_HIPRI 1, RWF_DSYNC 2, RWF_SYNC 4, RWF_APPEND
 * 0x10 and RWF_NOAPPEND 0x20 are accepted on a read and change nothing
 * about it; RWF_NOWAIT 8 on an empty pipe is EAGAIN (data present: read
 * normally); bits no kernel defines are EOPNOTSUPP, checked after the
 * offset and the descriptor, before the file type. */
#include "a_sys.h"

#define D "/tmp/lb_preadv2_flags"
#define SYS_preadv2 327
#define RWF_HIPRI 1
#define RWF_DSYNC 2
#define RWF_SYNC 4
#define RWF_NOWAIT 8
#define RWF_APPEND 0x10
#define RWF_NOAPPEND 0x20

static i64 preadv2_(i64 fd, struct a_iovec *iov, i64 cnt, i64 lo, i64 flags) {
    return syscall6(SYS_preadv2, fd, (i64)iov, cnt, lo, 0, flags);
}

int test_main(void) {
    char a[8];
    struct a_iovec v[1] = {{a, 4}};
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "abcdefgh", 0644), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    expect("hipri", preadv2_(fd, v, 1, 0, RWF_HIPRI) == 4 && mem_eq(a, "abcd", 4), 1);
    expect("dsync", preadv2_(fd, v, 1, 4, RWF_DSYNC) == 4 && mem_eq(a, "efgh", 4), 1);
    expect("sync", preadv2_(fd, v, 1, 1, RWF_SYNC) == 4 && mem_eq(a, "bcde", 4), 1);
    expect("append_on_read", preadv2_(fd, v, 1, 2, RWF_APPEND) == 4 && mem_eq(a, "cdef", 4), 1);
    expect("noappend_on_read", preadv2_(fd, v, 1, 3, RWF_NOAPPEND) == 4 && mem_eq(a, "defg", 4), 1);
    expect("several", preadv2_(fd, v, 1, 0, RWF_HIPRI | RWF_DSYNC | RWF_SYNC) == 4, 1);
    expect("position_still_0", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 0);
    expect("unknown_0x4000", preadv2_(fd, v, 1, 0, 0x4000), -EOPNOTSUPP);
    expect("unknown_top_bit", preadv2_(fd, v, 1, 0, (i64)0x80000000L), -EOPNOTSUPP);
    expect("unknown_with_known", preadv2_(fd, v, 1, 0, 0x40000000 | RWF_DSYNC), -EOPNOTSUPP);
    /* order: EBADF, then flags, then the offset */
    expect("bad_fd_vs_unknown", preadv2_(-1, v, 1, 0, 0x4000), -EBADF);
    expect("negative_offset_vs_unknown", preadv2_(fd, v, 1, -5, 0x4000), -EINVAL);
    i64 dir = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("unknown_vs_dir", preadv2_(dir, v, 1, 0, 0x4000), -EOPNOTSUPP);
    a_close(dir);

    /* RWF_NOWAIT on a pipe */
    i32 pp[2];
    syscall2(SYS_pipe2, pp, 0);
    expect("nowait_empty_pipe", preadv2_(pp[0], v, 1, -1, RWF_NOWAIT), -EAGAIN);
    expect("pipe_write", a_write(pp[1], "xy", 2), 2);
    expect("nowait_with_data", preadv2_(pp[0], v, 1, -1, RWF_NOWAIT), 2);
    expect("nowait_data", mem_eq(a, "xy", 2), 1);
    expect("nowait_empty_again", preadv2_(pp[0], v, 1, -1, RWF_NOWAIT), -EAGAIN);
    a_close(pp[1]);
    expect("nowait_writer_gone_eof", preadv2_(pp[0], v, 1, -1, RWF_NOWAIT), 0);

    a_close(fd);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
