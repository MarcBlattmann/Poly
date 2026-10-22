/* unlink updates the parent directory's mtime and ctime, and the ctime of
 * a file that keeps other names; the file's atime and mtime are unchanged.
 * (ctime is only checked not to go back: its clock is coarse.) */
#include "a_sys.h"

#define D "/tmp/c_unlink_times"
#define OLD 1000000000

static i64 u(const char *p) { return syscall1(SYS_unlink, p); }

static i64 set_old(const char *p) {
    struct a_timespec ts[2] = {{OLD, 0}, {OLD, 0}};
    return syscall4(SYS_utimensat, AT_FDCWD, p, ts, 0);
}

int test_main(void) {
    struct a_stat st, before;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_link", syscall2(SYS_link, D "/f", D "/g"), 0);
    expect("old_dir", set_old(D), 0);
    expect("old_file", set_old(D "/f"), 0);
    expect("stat_before", a_stat_path(D, &before), 0);
    expect("dir_mtime_old", before.st_mtime_sec, OLD);
    expect("unlink_f", u(D "/f"), 0);
    expect("stat_dir", a_stat_path(D, &st), 0);
    expect("dir_mtime_newer", st.st_mtime_sec > OLD, 1);
    expect("dir_ctime_not_older", st.st_ctime_sec > before.st_ctime_sec ||
                                      (st.st_ctime_sec == before.st_ctime_sec && st.st_ctime_nsec >= before.st_ctime_nsec),
           1);
    expect("stat_g", a_stat_path(D "/g", &st), 0);
    expect("file_mtime_kept", st.st_mtime_sec, OLD);
    expect("file_atime_kept", st.st_atime_sec, OLD);
    expect("file_ctime_not_old", st.st_ctime_sec > OLD, 1);
    /* A failed unlink changes nothing. */
    expect("old_dir_again", set_old(D), 0);
    expect("failed_unlink", u(D "/nope"), -ENOENT);
    expect("stat_dir_after_fail", a_stat_path(D, &st), 0);
    expect("dir_mtime_unchanged", st.st_mtime_sec, OLD);
    expect("cleanup_g", u(D "/g"), 0);
    expect("cleanup_dir", syscall1(SYS_rmdir, D), 0);
    return finish();
}
