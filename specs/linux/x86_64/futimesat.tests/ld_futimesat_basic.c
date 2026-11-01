/* futimesat(dirfd, path, times) (syscall 261) sets the access and
 * modification times of path (relative to dirfd, AT_FDCWD, or absolute
 * ignoring dirfd) from times[0] and times[1], struct timeval {tv_sec,
 * tv_usec}: the stored nanoseconds are tv_usec * 1000. Negative seconds
 * are allowed (before 1970). A symlink is followed (the target changes).
 * times NULL sets both to the current time. Returns 0.
 */
#include "a_sys.h"

#define SYS_futimesat 261
#define DIR "/tmp/ld_futimesat_basic"

static struct a_stat st;
static struct a_timeval tv[2];

static i64 fut(i64 d, const char *p, i64 as, i64 au, i64 ms, i64 mu) {
    tv[0].tv_sec = as;
    tv[0].tv_usec = au;
    tv[1].tv_sec = ms;
    tv[1].tv_usec = mu;
    return syscall3(SYS_futimesat, d, p, tv);
}

static void stat_of(const char *p) {
    memset(&st, 0, sizeof st);
    a_stat_path(p, &st);
}

static void cleanup(void) {
    a_unlink(DIR "/f");
    a_unlink(DIR "/l");
    a_rmdir(DIR "/sub");
    a_rmdir(DIR);
}

int test_main(void) {
    cleanup();
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    a_mkdir(DIR "/sub", 0755);
    expect("file", a_make_file(DIR "/f", "x", 0644), 0);
    syscall2(SYS_symlink, "f", DIR "/l");
    i64 d = a_open(DIR, O_RDONLY | O_DIRECTORY, 0);
    expect("dirfd_3", d, 3);

    expect("relative", fut(d, "f", 1000, 500000, 2000, 999999), 0);
    stat_of(DIR "/f");
    expect("atime_sec", st.st_atime_sec, 1000);
    expect("atime_nsec_usec_times_1000", st.st_atime_nsec, 500000000);
    expect("mtime_sec", st.st_mtime_sec, 2000);
    expect("mtime_nsec", st.st_mtime_nsec, 999999000);

    expect("negative_seconds", fut(d, "f", -5, 0, -1, 1), 0);
    stat_of(DIR "/f");
    expect("neg_atime", (i64)st.st_atime_sec, -5);
    expect("neg_mtime", (i64)st.st_mtime_sec, -1);
    expect("neg_mtime_nsec", st.st_mtime_nsec, 1000);
    expect("zero_times", fut(d, "f", 0, 0, 0, 0), 0);
    stat_of(DIR "/f");
    expect("zero_atime", st.st_atime_sec, 0);
    expect("zero_mtime", st.st_mtime_sec, 0);

    expect("follows_symlink", fut(d, "l", 3000, 1, 4000, 2), 0);
    stat_of(DIR "/f");
    expect("target_atime", st.st_atime_sec, 3000);
    expect("target_mtime", st.st_mtime_sec, 4000);

    expect("directory", fut(d, "sub", 5000, 0, 6000, 0), 0);
    stat_of(DIR "/sub");
    expect("dir_mtime", st.st_mtime_sec, 6000);
    expect("dot_is_dirfd_dir", fut(d, ".", 7000, 0, 7001, 0), 0);
    stat_of(DIR);
    expect("dot_mtime", st.st_mtime_sec, 7001);
    expect("absolute_ignores_bad_dirfd", fut(99, DIR "/f", 8000, 0, 8001, 0), 0);
    stat_of(DIR "/f");
    expect("absolute_mtime", st.st_mtime_sec, 8001);
    syscall1(SYS_chdir, DIR);
    expect("at_fdcwd", fut(AT_FDCWD, "f", 9000, 0, 9001, 0), 0);
    syscall1(SYS_chdir, "/");
    stat_of(DIR "/f");
    expect("at_fdcwd_mtime", st.st_mtime_sec, 9001);

    struct a_timespec before, after;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &before);
    expect("null_times_now", syscall3(SYS_futimesat, d, "f", 0), 0);
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &after);
    stat_of(DIR "/f");
    check("now_atime_in_range", (i64)st.st_atime_sec >= before.tv_sec - 1 && (i64)st.st_atime_sec <= after.tv_sec + 1, 0);
    check("now_mtime_in_range", (i64)st.st_mtime_sec >= before.tv_sec - 1 && (i64)st.st_mtime_sec <= after.tv_sec + 1, 0);
    check("now_atime_eq_mtime", st.st_atime_sec == st.st_mtime_sec && st.st_atime_nsec == st.st_mtime_nsec, 0);
    check("ctime_updated", (i64)st.st_ctime_sec >= before.tv_sec - 1, 0);

    a_close(d);
    cleanup();
    return finish();
}
