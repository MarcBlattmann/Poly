/* Repeated utime calls: the last one wins; explicit times may move either
 * way; a NULL call after explicit ones sets the current time. A directory's
 * own times are not changed by utime on an entry in it. */
#include "a_sys.h"

#define SYS_utime 132
#define D "/tmp/c_utime_repeat"

struct utimbuf_k {
    i64 actime, modtime;
};

int test_main(void) {
    struct a_stat st, dir_before, dir_after;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    struct utimbuf_k dt = {50, 60};
    expect("setup_dir_times", syscall2(SYS_utime, D, &dt), 0);
    a_stat_path(D, &dir_before);
    static const i64 seq[5][2] = {{1000000, 2000000}, {10, 20}, {3000000000LL, 5}, {5, 3000000000LL}, {7, 7}};
    static const char *const step[5] = {"step_1", "step_2", "step_3", "step_4", "step_5"};
    static const char *const vals[5] = {"step_1_values", "step_2_values", "step_3_values", "step_4_values",
                                        "step_5_values"};
    for (int i = 0; i < 5; i++) {
        struct utimbuf_k t = {seq[i][0], seq[i][1]};
        expect(step[i], syscall2(SYS_utime, D "/f", &t), 0);
        a_stat_path(D "/f", &st);
        check(vals[i], (i64)st.st_atime_sec == seq[i][0] && (i64)st.st_mtime_sec == seq[i][1],
              (i64)st.st_atime_sec);
    }
    expect("now", syscall2(SYS_utime, D "/f", 0), 0);
    a_stat_path(D "/f", &st);
    check("now_after_explicit", (i64)st.st_mtime_sec > 1600000000, (i64)st.st_mtime_sec);
    struct utimbuf_k back = {7, 7};
    expect("explicit_after_now", syscall2(SYS_utime, D "/f", &back), 0);
    a_stat_path(D "/f", &st);
    check("explicit_after_now_values", st.st_atime_sec == 7 && st.st_mtime_sec == 7, (i64)st.st_mtime_sec);
    a_stat_path(D, &dir_after);
    check("dir_times_untouched", dir_after.st_atime_sec == 50 && dir_after.st_mtime_sec == 60,
          (i64)dir_after.st_mtime_sec);
    check("dir_ctime_untouched",
          dir_after.st_ctime_sec == dir_before.st_ctime_sec && dir_after.st_ctime_nsec == dir_before.st_ctime_nsec,
          (i64)dir_after.st_ctime_nsec);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
