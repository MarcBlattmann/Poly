/* pwritev(2) writes several buffers in order at the given offset without
 * using or moving the file position; writing past the end extends the
 * file (a gap reads as zeros); the fifth argument (pos_h) is ignored on
 * x86-64; zero-length buffers are skipped; mtime and ctime change. */
#include "a_sys.h"

#define D "/tmp/lb_pwritev_basic"

static i64 pwritev_(i64 fd, struct a_iovec *iov, i64 cnt, i64 lo, i64 hi) {
    return syscall5(SYS_pwritev, fd, iov, cnt, lo, hi);
}

int test_main(void) {
    char back[64];
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    i64 fd = a_open(D "/f", O_RDWR | O_CREAT, 0644);
    expect("write_base", a_write(fd, "..........", 10), 10);

    struct a_iovec v[3] = {{"ab", 2}, {"cde", 3}, {"f", 1}};
    expect("gather_6", pwritev_(fd, v, 3, 2, 0), 6);
    expect("position_unchanged", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 10);
    expect("read_back", syscall4(SYS_pread64, fd, back, 64, 0), 10);
    expect("contents", mem_eq(back, "..abcdef..", 10), 1);
    expect("pos_h_ignored", pwritev_(fd, v, 1, 0, 0x7777) == 2, 1);
    expect("pos_h_result", syscall4(SYS_pread64, fd, back, 2, 0) == 2 && mem_eq(back, "ab", 2), 1);
    /* past the end: zeros in the gap */
    expect("past_end", pwritev_(fd, v, 2, 20, 0), 5);
    expect("size_25", syscall2(SYS_fstat, fd, &st) == 0 ? st.st_size : -1, 25);
    expect("gap_read", syscall4(SYS_pread64, fd, back, 64, 10), 15);
    expect("gap_zeros_then_data", mem_eq(back, "\0\0\0\0\0\0\0\0\0\0abcde", 15), 1);
    /* zero-length entries, count 0 */
    struct a_iovec z[3] = {{"X", 0}, {"Y", 1}, {0, 0}};
    expect("zero_length_entries", pwritev_(fd, z, 3, 0, 0), 1);
    expect("count_0", pwritev_(fd, v, 0, 0, 0), 0);
    expect("count_0_size", syscall2(SYS_fstat, fd, &st) == 0 ? st.st_size : -1, 25);
    /* times */
    struct a_timespec ts[2] = {{1000000000, 0}, {1000000000, 0}};
    expect("set_times", syscall4(SYS_utimensat, fd, 0, ts, 0), 0);
    expect("write_for_times", pwritev_(fd, v, 1, 0, 0), 2);
    expect("fstat_times", syscall2(SYS_fstat, fd, &st), 0);
    check("mtime_moved", st.st_mtime_sec > 1000000000, st.st_mtime_sec);
    expect("atime_kept", st.st_atime_sec, 1000000000);
    /* a write-only descriptor; /dev/null swallows everything */
    i64 wo = a_open(D "/f", O_WRONLY, 0);
    expect("wronly_fd", pwritev_(wo, v, 3, 0, 0), 6);
    i64 null = a_open("/dev/null", O_WRONLY, 0);
    expect("devnull", pwritev_(null, v, 3, 12345, 0), 6);

    a_close(null);
    a_close(wo);
    a_close(fd);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
