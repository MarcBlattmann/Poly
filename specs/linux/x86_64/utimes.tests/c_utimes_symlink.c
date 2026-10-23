/* utimess follows symbolic links: the target's times change, the link's own
 * do not. A dangling link is ENOENT, a link loop ELOOP. */
#include "a_sys.h"

#define D "/tmp/c_utimes_symlink"

/* sys/time.h: struct timeval { time_t tv_sec; suseconds_t tv_usec; } */
struct timeval_k {
    i64 tv_sec, tv_usec;
};

int test_main(void) {
    struct a_stat st, lst_before, lst_after;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/target", "", 0644), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "target", AT_FDCWD, D "/link"), 0);
    expect("setup_link2", syscall3(SYS_symlinkat, "link", AT_FDCWD, D "/link2"), 0);
    expect("setup_dangling", syscall3(SYS_symlinkat, "nowhere", AT_FDCWD, D "/dangling"), 0);
    expect("setup_loop_a", syscall3(SYS_symlinkat, "loop_b", AT_FDCWD, D "/loop_a"), 0);
    expect("setup_loop_b", syscall3(SYS_symlinkat, "loop_a", AT_FDCWD, D "/loop_b"), 0);
    a_lstat_path(D "/link", &lst_before);
    struct timeval_k t[2] = {{3333, 33}, {4444, 44}};
    expect("via_link", syscall2(SYS_utimes, D "/link", t), 0);
    expect("stat_target", a_stat_path(D "/target", &st), 0);
    check("target_atime", st.st_atime_sec == 3333 && st.st_atime_nsec == 33000, (i64)st.st_atime_sec);
    check("target_mtime", st.st_mtime_sec == 4444 && st.st_mtime_nsec == 44000, (i64)st.st_mtime_sec);
    a_lstat_path(D "/link", &lst_after);
    check("link_mtime_unchanged",
          lst_after.st_mtime_sec == lst_before.st_mtime_sec && lst_after.st_mtime_nsec == lst_before.st_mtime_nsec,
          (i64)lst_after.st_mtime_sec);
    struct timeval_k t2[2] = {{55, 0}, {66, 0}};
    expect("via_two_links", syscall2(SYS_utimes, D "/link2", t2), 0);
    a_stat_path(D "/target", &st);
    check("two_links_target", st.st_atime_sec == 55 && st.st_mtime_sec == 66, (i64)st.st_mtime_sec);
    expect("dangling", syscall2(SYS_utimes, D "/dangling", t), -ENOENT);
    expect("dangling_null", syscall2(SYS_utimes, D "/dangling", 0), -ENOENT);
    expect("loop", syscall2(SYS_utimes, D "/loop_a", t), -ELOOP);
    expect("loop_in_prefix", syscall2(SYS_utimes, D "/loop_a/x", t), -ELOOP);
    expect("cleanup_loop_b", a_unlink(D "/loop_b"), 0);
    expect("cleanup_loop_a", a_unlink(D "/loop_a"), 0);
    expect("cleanup_dangling", a_unlink(D "/dangling"), 0);
    expect("cleanup_link2", a_unlink(D "/link2"), 0);
    expect("cleanup_link", a_unlink(D "/link"), 0);
    expect("cleanup_target", a_unlink(D "/target"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
