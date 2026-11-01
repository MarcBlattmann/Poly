/* truncate(2) sets the size of the regular file named by path (links
 * followed): shrinking drops data, growing adds zero bytes; descriptors
 * see the new size, their offsets do not move; a size change moves mtime
 * and ctime. */
#include "a_sys.h"

#define D "/tmp/lb_truncate_basic"

static i64 truncate_(const char *p, i64 len) { return syscall2(SYS_truncate, p, len); }
static i64 size_of(const char *p) {
    struct a_stat st;
    if (a_stat_path(p, &st)) return -1;
    return st.st_size;
}

int test_main(void) {
    struct a_stat st, st2;
    char c[32];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "0123456789", 0644), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    check("open", fd >= 0, fd);
    expect("seek_8", syscall3(SYS_lseek, fd, 8, SEEK_SET), 8);

    expect("shrink_4", truncate_(D "/f", 4), 0);
    expect("size_4", size_of(D "/f"), 4);
    expect("fd_sees_size", syscall2(SYS_fstat, fd, &st) == 0 && st.st_size == 4, 1);
    expect("offset_unchanged", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 8);
    expect("read_past_end", a_read(fd, c, 32), 0);
    expect("grow_12", truncate_(D "/f", 12), 0);
    expect("size_12", size_of(D "/f"), 12);
    expect("pread_all", syscall4(SYS_pread64, fd, c, 32, 0), 12);
    expect("kept_prefix", mem_eq(c, "0123", 4), 1);
    expect("zero_filled", mem_eq(c + 4, "\0\0\0\0\0\0\0\0", 8), 1);
    expect("to_zero", truncate_(D "/f", 0), 0);
    expect("size_0", size_of(D "/f"), 0);
    expect("big_sparse", truncate_(D "/f", 1L << 40), 0);
    expect("size_1T", size_of(D "/f"), 1L << 40);
    expect("pread_far", syscall4(SYS_pread64, fd, c, 4, (1L << 40) - 2), 2);
    expect("far_zeros", c[0] == 0 && c[1] == 0, 1);
    expect("back_to_10", truncate_(D "/f", 10), 0);

    /* times: a size change moves mtime and ctime */
    struct a_timespec ts[2] = {{1000000000, 0}, {1000000000, 0}};
    expect("set_times", syscall4(SYS_utimensat, AT_FDCWD, D "/f", ts, 0), 0);
    expect("stat0", a_stat_path(D "/f", &st), 0);
    struct a_timespec pause = {0, 20000000};
    syscall2(SYS_nanosleep, &pause, 0);
    expect("shrink_for_times", truncate_(D "/f", 5), 0);
    expect("stat1", a_stat_path(D "/f", &st2), 0);
    check("mtime_moved", st2.st_mtime_sec > 1000000000, st2.st_mtime_sec);
    check("ctime_moved", st2.st_ctime_sec > st.st_ctime_sec ||
                             (st2.st_ctime_sec == st.st_ctime_sec && st2.st_ctime_nsec > st.st_ctime_nsec), 0);
    expect("atime_kept", st2.st_atime_sec, 1000000000);
    /* the same size again succeeds (whether times move depends on the file
     * system: see the tmpfs test) */
    expect("same_size", truncate_(D "/f", 5), 0);

    /* symbolic links are followed; relative paths */
    expect("setup_link", syscall2(SYS_symlink, "f", D "/l"), 0);
    expect("via_link", truncate_(D "/l", 3), 0);
    expect("target_size_3", size_of(D "/f"), 3);
    expect("link_unchanged", a_lstat_path(D "/l", &st) == 0 && st.st_size == 1, 1);
    expect("chdir", syscall1(SYS_chdir, D), 0);
    expect("relative", truncate_("f", 7), 0);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("size_7", size_of(D "/f"), 7);
    /* the file need not be open, and an open O_RDONLY descriptor does not matter */
    expect("close", a_close(fd), 0);

    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
