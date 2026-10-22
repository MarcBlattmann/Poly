/* truncate updates the file's mtime and ctime, also when the length is
 * the size the file already has; atime does not change. A failed call
 * (negative length) changes none of them. */
#include "a_sys.h"

#define F "/tmp/a_truncate_times"

static struct a_timespec old[2] = {{100, 0}, {200, 0}};

int test_main(void) {
    struct a_stat st;
    expect("setup_file", a_make_file(F, "0123456789", 0644), 0);

    expect("setup_times", syscall4(SYS_utimensat, AT_FDCWD, F, old, 0), 0);
    expect("shrink", syscall2(SYS_truncate, F, 4), 0);
    a_stat_path(F, &st);
    check("shrink_mtime_changed", st.st_mtime_sec > 200, st.st_mtime_sec);
    check("shrink_ctime_changed", st.st_ctime_sec > 200, st.st_ctime_sec);
    expect("shrink_atime_kept", st.st_atime_sec, 100);
    check("shrink_mtime_eq_ctime",
          st.st_mtime_sec == st.st_ctime_sec && st.st_mtime_nsec == st.st_ctime_nsec, 0);

    expect("setup_times2", syscall4(SYS_utimensat, AT_FDCWD, F, old, 0), 0);
    expect("grow", syscall2(SYS_truncate, F, 40), 0);
    a_stat_path(F, &st);
    check("grow_mtime_changed", st.st_mtime_sec > 200, st.st_mtime_sec);
    expect("grow_atime_kept", st.st_atime_sec, 100);

    expect("setup_times3", syscall4(SYS_utimensat, AT_FDCWD, F, old, 0), 0);
    expect("same_size", syscall2(SYS_truncate, F, 40), 0);
    a_stat_path(F, &st);
    check("same_size_mtime_changed", st.st_mtime_sec > 200, st.st_mtime_sec);
    check("same_size_ctime_changed", st.st_ctime_sec > 200, st.st_ctime_sec);
    expect("same_size_atime_kept", st.st_atime_sec, 100);

    expect("setup_times4", syscall4(SYS_utimensat, AT_FDCWD, F, old, 0), 0);
    a_stat_path(F, &st);
    u64 ctime = st.st_ctime_sec, ctime_ns = st.st_ctime_nsec;
    expect("negative_fails", syscall2(SYS_truncate, F, -1), -EINVAL);
    a_stat_path(F, &st);
    expect("failed_mtime_kept", st.st_mtime_sec, 200);
    check("failed_ctime_kept", st.st_ctime_sec == ctime && st.st_ctime_nsec == ctime_ns, st.st_ctime_sec);
    expect("failed_size_kept", st.st_size, 40);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
