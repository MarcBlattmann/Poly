/* chmod changes the file's ctime to the current time, also when the mode
 * stays the same; mtime, atime, size and contents do not change, nor do the
 * directory's times. A failed chmod changes nothing. */
#include "a_sys.h"

#define D "/tmp/a_chmod_times"

static void nap(void) {
    struct a_timespec t = {0, 30000000};
    syscall2(SYS_nanosleep, &t, 0);
}

static int later(u64 s1, u64 n1, u64 s0, u64 n0) { return s1 > s0 || (s1 == s0 && n1 > n0); }
static int same(u64 s1, u64 n1, u64 s0, u64 n0) { return s1 == s0 && n1 == n0; }

int test_main(void) {
    struct a_stat a, b, da, db;
    struct a_timespec now0, now1;
    char text[8];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "hello", 0644), 0);
    expect("stat_before", a_stat_path(D "/f", &a), 0);
    expect("dir_stat_before", a_stat_path(D, &da), 0);
    nap();
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &now0);
    expect("chmod", syscall2(SYS_chmod, D "/f", 0600), 0);
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &now1);
    expect("stat_after", a_stat_path(D "/f", &b), 0);
    check("ctime_advanced", later(b.st_ctime_sec, b.st_ctime_nsec, a.st_ctime_sec, a.st_ctime_nsec), (i64)b.st_ctime_sec);
    check("ctime_is_now", b.st_ctime_sec + 1 >= (u64)now0.tv_sec && b.st_ctime_sec <= (u64)now1.tv_sec, (i64)b.st_ctime_sec);
    check("mtime_kept", same(b.st_mtime_sec, b.st_mtime_nsec, a.st_mtime_sec, a.st_mtime_nsec), (i64)b.st_mtime_sec);
    check("atime_kept", same(b.st_atime_sec, b.st_atime_nsec, a.st_atime_sec, a.st_atime_nsec), (i64)b.st_atime_sec);
    expect("size_kept", b.st_size, 5);
    expect("ino_kept", b.st_ino == a.st_ino, 1);
    expect("nlink_kept", b.st_nlink, 1);
    expect("dir_stat_after", a_stat_path(D, &db), 0);
    check("dir_mtime_kept", same(db.st_mtime_sec, db.st_mtime_nsec, da.st_mtime_sec, da.st_mtime_nsec), 0);
    check("dir_ctime_kept", same(db.st_ctime_sec, db.st_ctime_nsec, da.st_ctime_sec, da.st_ctime_nsec), 0);

    /* Same mode again: ctime still moves. */
    nap();
    expect("chmod_same_mode", syscall2(SYS_chmod, D "/f", 0600), 0);
    expect("stat_same", a_stat_path(D "/f", &a), 0);
    check("same_mode_ctime_advanced", later(a.st_ctime_sec, a.st_ctime_nsec, b.st_ctime_sec, b.st_ctime_nsec), 0);

    /* A failed chmod (a missing name below a file) leaves it alone. */
    nap();
    expect("failed_chmod", syscall2(SYS_chmod, D "/f/x", 0700), -ENOTDIR);
    expect("stat_failed", a_stat_path(D "/f", &b), 0);
    check("failed_ctime_kept", same(b.st_ctime_sec, b.st_ctime_nsec, a.st_ctime_sec, a.st_ctime_nsec), 0);
    expect("failed_mode_kept", b.st_mode, S_IFREG | 0600);

    i64 fd = a_open(D "/f", O_RDONLY, 0);
    expect("contents_kept", a_read(fd, text, sizeof text), 5);
    check("contents_text", mem_eq(text, "hello", 5), 0);
    a_close(fd);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
