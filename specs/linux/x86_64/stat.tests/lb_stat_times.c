/* stat(2) timestamps and sizes: times set by utimensat are reported to
 * the nanosecond; a write moves mtime and ctime; st_size follows writes
 * and truncation; st_blocks counts 512-byte units actually allocated (a
 * hole has none); stat itself changes nothing. */
#include "a_sys.h"

#define D "/tmp/lb_stat_times"

static i64 stat_(const char *p, void *st) { return syscall2(SYS_stat, p, st); }

int test_main(void) {
    struct a_stat st, st2;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "", 0644), 0);
    struct a_timespec ts[2] = {{1234567890, 123456789}, {987654321, 999999999}};
    expect("utimensat", syscall4(SYS_utimensat, AT_FDCWD, D "/f", ts, 0), 0);
    expect("stat", stat_(D "/f", &st), 0);
    expect("atime_sec", st.st_atime_sec, 1234567890);
    expect("atime_nsec", st.st_atime_nsec, 123456789);
    expect("mtime_sec", st.st_mtime_sec, 987654321);
    expect("mtime_nsec", st.st_mtime_nsec, 999999999);
    /* negative and pre-1970 times */
    struct a_timespec neg[2] = {{-1, 0}, {-86400, 500}};
    expect("utimensat_neg", syscall4(SYS_utimensat, AT_FDCWD, D "/f", neg, 0), 0);
    expect("stat_neg", stat_(D "/f", &st), 0);
    expect("atime_minus1", st.st_atime_sec, -1);
    expect("mtime_minus_day", st.st_mtime_sec, -86400);
    expect("mtime_nsec_500", st.st_mtime_nsec, 500);

    /* stat changes nothing */
    expect("stat_again", stat_(D "/f", &st2), 0);
    expect("identical", mem_eq(&st, &st2, sizeof st), 1);

    /* writes move mtime and ctime */
    struct a_timespec pause = {0, 20000000};
    syscall2(SYS_nanosleep, &pause, 0);
    i64 fd = a_open(D "/f", O_WRONLY, 0);
    expect("write", a_write(fd, "hello", 5), 5);
    expect("stat_written", stat_(D "/f", &st2), 0);
    check("mtime_moved", st2.st_mtime_sec > 0, st2.st_mtime_sec);
    check("ctime_moved", st2.st_ctime_sec > st.st_ctime_sec ||
                             (st2.st_ctime_sec == st.st_ctime_sec && st2.st_ctime_nsec > st.st_ctime_nsec), 0);
    expect("atime_kept", st2.st_atime_sec, -1);
    expect("size_5", st2.st_size, 5);

    /* sizes and blocks */
    expect("ftruncate_0", syscall2(SYS_ftruncate, fd, 0), 0);
    expect("stat_empty", stat_(D "/f", &st), 0);
    expect("empty_size", st.st_size, 0);
    expect("empty_blocks", st.st_blocks, 0);
    expect("ftruncate_hole", syscall2(SYS_ftruncate, fd, 10 * 1024 * 1024), 0);
    expect("stat_hole", stat_(D "/f", &st), 0);
    expect("hole_size", st.st_size, 10 * 1024 * 1024);
    expect("hole_blocks", st.st_blocks, 0);
    char page[4096];
    for (int i = 0; i < 4096; i++) page[i] = 'p';
    expect("pwrite_page", syscall4(SYS_pwrite64, fd, page, 4096, 0), 4096);
    expect("stat_page", stat_(D "/f", &st), 0);
    check("page_blocks_8_or_more", st.st_blocks >= 8, st.st_blocks);
    expect("page_size_unchanged", st.st_size, 10 * 1024 * 1024);
    a_close(fd);

    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
