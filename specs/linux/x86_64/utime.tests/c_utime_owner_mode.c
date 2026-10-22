/* The owner of a file may change its times whatever the file's mode, both
 * with explicit times and with NULL (the current time). */
#include "a_sys.h"

#define SYS_utime 132
#define D "/tmp/c_utime_owner_mode"

struct utimbuf_k {
    i64 actime, modtime;
};

int test_main(void) {
    struct a_stat st;
    struct utimbuf_k t = {4242, 2424};
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_ro", a_make_file(D "/ro", "", 0444), 0);
    expect("setup_none", a_make_file(D "/none", "", 0000), 0);
    expect("ro_explicit", syscall2(SYS_utime, D "/ro", &t), 0);
    a_stat_path(D "/ro", &st);
    check("ro_explicit_values", st.st_atime_sec == 4242 && st.st_mtime_sec == 2424, (i64)st.st_mtime_sec);
    expect("ro_now", syscall2(SYS_utime, D "/ro", 0), 0);
    a_stat_path(D "/ro", &st);
    check("ro_now_values", st.st_mtime_sec > 2424 && st.st_atime_sec == st.st_mtime_sec, (i64)st.st_mtime_sec);
    expect("none_explicit", syscall2(SYS_utime, D "/none", &t), 0);
    a_stat_path(D "/none", &st);
    check("none_explicit_values", st.st_atime_sec == 4242 && st.st_mtime_sec == 2424, (i64)st.st_mtime_sec);
    expect("none_now", syscall2(SYS_utime, D "/none", 0), 0);
    a_stat_path(D "/none", &st);
    check("none_now_values", st.st_mtime_sec > 2424, (i64)st.st_mtime_sec);
    check("none_mode_kept", (st.st_mode & 07777) == 0, st.st_mode);
    expect("setup_chmod_dir", syscall2(SYS_chmod, D, 0500), 0);
    expect("dir_readonly_explicit", syscall2(SYS_utime, D, &t), 0);
    a_stat_path(D, &st);
    check("dir_readonly_values", st.st_atime_sec == 4242 && st.st_mtime_sec == 2424, (i64)st.st_mtime_sec);
    expect("setup_chmod_dir_back", syscall2(SYS_chmod, D, 0700), 0);
    expect("cleanup_ro", a_unlink(D "/ro"), 0);
    expect("cleanup_none", a_unlink(D "/none"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
