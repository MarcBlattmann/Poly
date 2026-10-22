/* creat(2) and timestamps: truncating an existing file updates its mtime
 * and leaves atime alone (also when it was already empty); creating a new name updates
 * the directory's mtime; a failed creat changes nothing. */
#include "a_sys.h"

#define SYS_creat 85
#define D "/tmp/a_creat_times"

static i64 cr(const char *p, i64 mode) { return syscall2(SYS_creat, p, mode); }

static i64 set_old(const char *p) {
    struct a_timespec ts[2];
    ts[0].tv_sec = 1000000000;
    ts[0].tv_nsec = 0;
    ts[1].tv_sec = 1000000000;
    ts[1].tv_nsec = 0;
    return syscall4(SYS_utimensat, AT_FDCWD, p, ts, 0);
}

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_full", a_make_file(D "/full", "data", 0644), 0);
    expect("setup_empty", a_make_file(D "/empty", "", 0644), 0);
    expect("old_full", set_old(D "/full"), 0);
    expect("old_empty", set_old(D "/empty"), 0);
    expect("old_dir", set_old(D), 0);

    i64 fd = cr(D "/full", 0644);
    expect("creat_full", fd, 3);
    a_close(fd);
    expect("stat_full", a_stat_path(D "/full", &st), 0);
    check("full_mtime_updated", st.st_mtime_sec > 1000000000, st.st_mtime_sec);
    check("full_atime_kept", st.st_atime_sec == 1000000000, st.st_atime_sec);

    fd = cr(D "/empty", 0644);
    expect("creat_empty", fd, 3);
    a_close(fd);
    expect("stat_empty", a_stat_path(D "/empty", &st), 0);
    check("empty_mtime_updated", st.st_mtime_sec > 1000000000, st.st_mtime_sec);

    /* Existing names do not change the directory. */
    expect("stat_dir_1", a_stat_path(D, &st), 0);
    check("dir_mtime_kept_for_existing", st.st_mtime_sec == 1000000000, st.st_mtime_sec);

    fd = cr(D "/new", 0644);
    expect("creat_new", fd, 3);
    a_close(fd);
    expect("stat_dir_2", a_stat_path(D, &st), 0);
    check("dir_mtime_updated_for_new", st.st_mtime_sec > 1000000000, st.st_mtime_sec);

    /* A failed creat (EISDIR) leaves the directory alone. */
    expect("old_dir_again", set_old(D), 0);
    expect("eisdir", cr(D "/", 0644), -EISDIR);
    expect("stat_dir_3", a_stat_path(D, &st), 0);
    check("dir_mtime_kept_after_failure", st.st_mtime_sec == 1000000000, st.st_mtime_sec);

    expect("cleanup_full", a_unlink(D "/full"), 0);
    expect("cleanup_empty", a_unlink(D "/empty"), 0);
    expect("cleanup_new", a_unlink(D "/new"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
