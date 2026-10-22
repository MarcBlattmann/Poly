/* link: the file's ctime and both directories' mtime and ctime move
 * forward; the file's mtime, atime and contents do not change. A failed
 * link changes no time. */
#include "a_sys.h"

#define D "/tmp/a_link_times"

static int after(u64 s1, u64 n1, u64 s0, u64 n0) { return s1 > s0 || (s1 == s0 && n1 > n0); }
static int same(u64 s1, u64 n1, u64 s0, u64 n0) { return s1 == s0 && n1 == n0; }

static void pause_ms(int ms) {
    i64 ts[2] = {0, (i64)ms * 1000000};
    syscall2(SYS_nanosleep, ts, 0);
}

int test_main(void) {
    struct a_stat f0, f1, d0, d1, s0, s1;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_f", a_make_file(D "/f", "abc", 0644), 0);
    pause_ms(30);
    a_stat_path(D "/f", &f0);
    a_stat_path(D, &d0);
    a_stat_path(D "/sub", &s0);
    pause_ms(30);
    expect("link", syscall2(SYS_link, D "/f", D "/sub/g"), 0);
    a_stat_path(D "/f", &f1);
    a_stat_path(D, &d1);
    a_stat_path(D "/sub", &s1);
    check("file_ctime_moves", after(f1.st_ctime_sec, f1.st_ctime_nsec, f0.st_ctime_sec, f0.st_ctime_nsec), 0);
    check("file_mtime_same", same(f1.st_mtime_sec, f1.st_mtime_nsec, f0.st_mtime_sec, f0.st_mtime_nsec), 0);
    check("file_atime_same", same(f1.st_atime_sec, f1.st_atime_nsec, f0.st_atime_sec, f0.st_atime_nsec), 0);
    expect("file_size_same", f1.st_size, 3);
    check("new_dir_mtime_moves", after(s1.st_mtime_sec, s1.st_mtime_nsec, s0.st_mtime_sec, s0.st_mtime_nsec), 0);
    check("new_dir_ctime_moves", after(s1.st_ctime_sec, s1.st_ctime_nsec, s0.st_ctime_sec, s0.st_ctime_nsec), 0);
    check("old_dir_mtime_same", same(d1.st_mtime_sec, d1.st_mtime_nsec, d0.st_mtime_sec, d0.st_mtime_nsec), 0);
    check("old_dir_ctime_same", same(d1.st_ctime_sec, d1.st_ctime_nsec, d0.st_ctime_sec, d0.st_ctime_nsec), 0);
    check("new_dir_nlink_same", s1.st_nlink == s0.st_nlink, s1.st_nlink);
    pause_ms(30);
    expect("failed_link", syscall2(SYS_link, D "/f", D "/sub/g"), -EEXIST);
    a_stat_path(D "/f", &f0);
    a_stat_path(D "/sub", &s0);
    check("failed_file_ctime_same", same(f0.st_ctime_sec, f0.st_ctime_nsec, f1.st_ctime_sec, f1.st_ctime_nsec), 0);
    check("failed_dir_mtime_same", same(s0.st_mtime_sec, s0.st_mtime_nsec, s1.st_mtime_sec, s1.st_mtime_nsec), 0);
    expect("cleanup_g", a_unlink(D "/sub/g"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
