/* preadv(2) reads into several buffers in order, starting at the given
 * offset, without using or moving the file position. The offset is the
 * fourth argument (64 bits); the fifth (pos_h) is ignored on x86-64. It
 * returns the bytes read: short at the end of the file, 0 past it;
 * zero-length buffers are skipped; up to 1024 buffers. */
#include "a_sys.h"

#define D "/tmp/lb_preadv_basic"

static i64 preadv_(i64 fd, struct a_iovec *iov, i64 cnt, i64 lo, i64 hi) {
    return syscall5(SYS_preadv, fd, iov, cnt, lo, hi);
}

static char many[1024];
static struct a_iovec iovs[1024];

int test_main(void) {
    char a[4], b[3], c[5];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "0123456789abcdef", 0644), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    expect("seek_7", syscall3(SYS_lseek, fd, 7, SEEK_SET), 7);

    struct a_iovec v[3] = {{a, 4}, {b, 3}, {c, 5}};
    expect("scatter_12", preadv_(fd, v, 3, 2, 0), 12);
    expect("buf_a", mem_eq(a, "2345", 4), 1);
    expect("buf_b", mem_eq(b, "678", 3), 1);
    expect("buf_c", mem_eq(c, "9abcd", 5), 1);
    expect("position_unchanged", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 7);
    expect("pos_h_ignored", preadv_(fd, v, 1, 0, 0x12345678) == 4 && mem_eq(a, "0123", 4), 1);
    expect("pos_h_all_ones", preadv_(fd, v, 1, 4, -1) == 4 && mem_eq(a, "4567", 4), 1);
    /* short read at the end, 0 past it */
    expect("short_at_end", preadv_(fd, v, 3, 10, 0), 6);
    expect("short_a", mem_eq(a, "abcd", 4), 1);
    expect("short_b", mem_eq(b, "ef", 2), 1);
    expect("at_end", preadv_(fd, v, 3, 16, 0), 0);
    expect("past_end", preadv_(fd, v, 3, 1000000, 0), 0);
    /* zero-length entries and count 0 */
    struct a_iovec z[4] = {{a, 0}, {b, 3}, {0, 0}, {c, 2}};
    expect("zero_length_entries", preadv_(fd, z, 4, 0, 0), 5);
    expect("z_b", mem_eq(b, "012", 3), 1);
    expect("z_c", mem_eq(c, "34", 2), 1);
    expect("count_0", preadv_(fd, v, 0, 0, 0), 0);
    expect("count_0_null_iov", preadv_(fd, 0, 0, 0, 0), 0);
    /* 1024 one-byte buffers */
    for (int i = 0; i < 1024; i++) iovs[i] = (struct a_iovec){many + i, 1};
    expect("iov_1024", preadv_(fd, iovs, 1024, 0, 0), 16);
    expect("iov_1024_data", mem_eq(many, "0123456789abcdef", 16), 1);
    /* a write-and-read descriptor; the position stays after a big read */
    i64 rw = a_open(D "/f", O_RDWR, 0);
    expect("rw_fd", preadv_(rw, v, 1, 12, 0) == 4 && mem_eq(a, "cdef", 4), 1);
    expect("rw_position_0", syscall3(SYS_lseek, rw, 0, SEEK_CUR), 0);
    /* /dev/zero gives zeros at any offset, /dev/null 0 */
    i64 zero = a_open("/dev/zero", O_RDONLY, 0);
    a[0] = 'x';
    expect("devzero", preadv_(zero, v, 1, 12345, 0) == 4 && a[0] == 0 && a[3] == 0, 1);
    i64 null = a_open("/dev/null", O_RDONLY, 0);
    expect("devnull", preadv_(null, v, 3, 0, 0), 0);

    a_close(null);
    a_close(zero);
    a_close(rw);
    a_close(fd);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
