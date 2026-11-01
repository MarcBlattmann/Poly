/* fallocate(2) modes every common file system supports: 0 allocates and
 * extends the size (new bytes read as zeros), FALLOC_FL_KEEP_SIZE (1)
 * allocates without changing the size, FALLOC_FL_PUNCH_HOLE|KEEP_SIZE (3)
 * makes a range read as zeros and frees whole blocks; the file offset
 * never moves; mtime/ctime change when the size changes. */
#include "a_sys.h"

#define D "/tmp/lb_fallocate_modes"
#define SYS_fallocate 285
#define KEEP 1
#define PUNCH 2

static i64 falloc(i64 fd, i64 mode, i64 off, i64 len) { return syscall4(SYS_fallocate, fd, mode, off, len); }

static char page[16384], back[16384];

int test_main(void) {
    struct a_stat st, st2;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    i64 fd = a_open(D "/f", O_RDWR | O_CREAT, 0644);
    check("open", fd >= 0, fd);
    expect("write_hello", a_write(fd, "hello", 5), 5);

    /* mode 0 past the end: the size grows, the new bytes are zero */
    expect("mode0_extend", falloc(fd, 0, 0, 16384), 0);
    expect("fstat1", syscall2(SYS_fstat, fd, &st), 0);
    expect("size_16384", st.st_size, 16384);
    check("blocks_allocated", st.st_blocks >= 32, st.st_blocks);
    expect("offset_unchanged", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 5);
    expect("pread_all", syscall4(SYS_pread64, fd, back, 16384, 0), 16384);
    expect("data_kept", mem_eq(back, "hello", 5), 1);
    int nonzero = 0;
    for (int i = 5; i < 16384; i++) nonzero += back[i] != 0;
    expect("rest_zero", nonzero, 0);
    /* mode 0 inside the file: nothing visible changes */
    expect("mode0_inside", falloc(fd, 0, 0, 100), 0);
    expect("fstat2", syscall2(SYS_fstat, fd, &st), 0);
    expect("size_still_16384", st.st_size, 16384);
    expect("data_still_kept", syscall4(SYS_pread64, fd, back, 5, 0) == 5 && mem_eq(back, "hello", 5), 1);
    /* offset + len just past the end */
    expect("mode0_one_past", falloc(fd, 0, 16000, 385), 0);
    expect("size_16385", syscall2(SYS_fstat, fd, &st) == 0 ? st.st_size : -1, 16385);

    /* KEEP_SIZE past the end: blocks, no size */
    expect("ftruncate_8192", syscall2(SYS_ftruncate, fd, 8192), 0);
    expect("fstat3", syscall2(SYS_fstat, fd, &st), 0);
    expect("keep_size", falloc(fd, KEEP, 8192, 65536), 0);
    expect("fstat4", syscall2(SYS_fstat, fd, &st2), 0);
    expect("keep_size_unchanged", st2.st_size, 8192);
    check("keep_size_blocks_grew", st2.st_blocks > st.st_blocks, st2.st_blocks);
    expect("read_at_end_eof", syscall4(SYS_pread64, fd, back, 10, 8192), 0);

    /* PUNCH_HOLE|KEEP_SIZE: zeros, the size stays, whole blocks freed */
    for (int i = 0; i < 16384; i++) page[i] = 'x';
    expect("ftruncate_0", syscall2(SYS_ftruncate, fd, 0), 0);
    expect("write_16k", syscall4(SYS_pwrite64, fd, page, 16384, 0), 16384);
    expect("fstat5", syscall2(SYS_fstat, fd, &st), 0);
    expect("punch", falloc(fd, KEEP | PUNCH, 4096, 8192), 0);
    expect("fstat6", syscall2(SYS_fstat, fd, &st2), 0);
    expect("punch_size_kept", st2.st_size, 16384);
    check("punch_blocks_freed", st2.st_blocks < st.st_blocks, st2.st_blocks);
    expect("pread_punched", syscall4(SYS_pread64, fd, back, 16384, 0), 16384);
    int bad = 0;
    for (int i = 0; i < 16384; i++) bad += back[i] != ((i >= 4096 && i < 12288) ? 0 : 'x');
    expect("punched_range_zero_rest_kept", bad, 0);
    /* a partial-block punch zeroes just those bytes */
    expect("punch_partial", falloc(fd, KEEP | PUNCH, 1, 2), 0);
    expect("pread_partial", syscall4(SYS_pread64, fd, back, 4, 0), 4);
    expect("partial_bytes", back[0] == 'x' && back[1] == 0 && back[2] == 0 && back[3] == 'x', 1);
    /* a punch past the end does nothing */
    expect("punch_past_end", falloc(fd, KEEP | PUNCH, 20000, 4096), 0);
    expect("size_after_punch_past", syscall2(SYS_fstat, fd, &st) == 0 ? st.st_size : -1, 16384);

    /* times move when mode 0 grows the file */
    struct a_timespec ts[2] = {{1000000000, 0}, {1000000000, 0}};
    expect("set_times", syscall4(SYS_utimensat, fd, 0, ts, 0), 0);
    expect("grow_for_times", falloc(fd, 0, 0, 20000), 0);
    expect("fstat7", syscall2(SYS_fstat, fd, &st), 0);
    check("mtime_moved", st.st_mtime_sec > 1000000000, st.st_mtime_sec);
    /* an O_APPEND descriptor is fine (it is open for writing) */
    i64 ap = a_open(D "/f", O_WRONLY | O_APPEND, 0);
    expect("append_fd", falloc(ap, 0, 0, 30000), 0);
    expect("append_fd_size", syscall2(SYS_fstat, fd, &st) == 0 ? st.st_size : -1, 30000);
    a_close(ap);

    expect("close", a_close(fd), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
