/* rmdir(2) removes an empty directory: its parent loses a link and gets
 * new mtime/ctime; trailing slashes are fine; relative paths start at the
 * working directory; the name is free again at once. */
#include "a_sys.h"

#define D "/tmp/lb_rmdir_basic"

static i64 rmdir_(const char *p) { return syscall1(SYS_rmdir, p); }
static i64 nlink_of(const char *p) {
    struct a_stat st;
    if (a_lstat_path(p, &st)) return -1;
    return (i64)st.st_nlink;
}

int test_main(void) {
    struct a_stat st, st2;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_a", a_mkdir(D "/a", 0755), 0);
    expect("setup_b", a_mkdir(D "/b", 0755), 0);
    expect("setup_c", a_mkdir(D "/c", 0700), 0);
    expect("setup_d", a_mkdir(D "/d", 0), 0);
    expect("parent_nlink_6", nlink_of(D), 6);

    expect("rmdir_a", rmdir_(D "/a"), 0);
    expect("a_gone", a_stat_path(D "/a", &st), -ENOENT);
    expect("parent_nlink_5", nlink_of(D), 5);
    expect("rmdir_a_again", rmdir_(D "/a"), -ENOENT);
    expect("rmdir_trailing_slash", rmdir_(D "/b/"), 0);
    expect("rmdir_slashes", rmdir_(D "//c//"), 0);
    /* the directory's own mode does not matter */
    expect("rmdir_mode_0", rmdir_(D "/d"), 0);
    expect("parent_nlink_2", nlink_of(D), 2);
    /* the name can be used again */
    expect("recreate", a_mkdir(D "/a", 0755), 0);
    expect("recreate_file", a_unlink(D "/a") == -EISDIR && rmdir_(D "/a") == 0, 1);
    expect("file_same_name", a_make_file(D "/a", "x", 0644), 0);
    expect("file_cleanup", a_unlink(D "/a"), 0);

    /* relative paths */
    expect("setup_rel", a_mkdir(D "/rel", 0755), 0);
    expect("setup_rel_x", a_mkdir(D "/rel/x", 0755), 0);
    expect("setup_rel_y", a_mkdir(D "/rel/y", 0755), 0);
    expect("chdir", syscall1(SYS_chdir, D "/rel"), 0);
    expect("relative", rmdir_("x"), 0);
    expect("relative_dotdot", rmdir_("../rel/y"), 0);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("rel_nlink_2", nlink_of(D "/rel"), 2);

    /* the parent's times change */
    expect("setup_t", a_mkdir(D "/rel/t", 0755), 0);
    struct a_timespec ts[2] = {{1000000000, 0}, {1000000000, 0}};
    expect("set_times", syscall4(SYS_utimensat, AT_FDCWD, D "/rel", ts, 0), 0);
    expect("stat0", a_stat_path(D "/rel", &st), 0);
    struct a_timespec pause = {0, 20000000};
    syscall2(SYS_nanosleep, &pause, 0);
    expect("rmdir_t", rmdir_(D "/rel/t"), 0);
    expect("stat1", a_stat_path(D "/rel", &st2), 0);
    check("parent_mtime_changed", st2.st_mtime_sec > 1000000000, 0);
    check("parent_ctime_changed",
          st2.st_ctime_sec > st.st_ctime_sec || (st2.st_ctime_sec == st.st_ctime_sec && st2.st_ctime_nsec > st.st_ctime_nsec),
          0);
    expect("parent_atime_kept", st2.st_atime_sec, 1000000000);

    expect("cleanup_rel", rmdir_(D "/rel"), 0);
    expect("cleanup_rmdir", rmdir_(D), 0);
    expect("gone", a_stat_path(D, &st), -ENOENT);
    return finish();
}
