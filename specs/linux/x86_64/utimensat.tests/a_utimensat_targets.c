/* utimensat: a NULL path sets the times of the descriptor's file;
 * AT_SYMLINK_NOFOLLOW sets a link's own times; errors for missing paths,
 * bad descriptors and flags; utimes (microseconds). */
#include "a_sys.h"

#define D "/tmp/a_utimensat_targets"

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    struct a_timespec t[2] = {{500, 5}, {600, 6}};
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    expect("null_path_fd", syscall4(SYS_utimensat, fd, 0, t, 0), 0);
    syscall2(SYS_fstat, fd, &st);
    check("fd_times", st.st_atime_sec == 500 && st.st_mtime_sec == 600 && st.st_mtime_nsec == 6, st.st_mtime_sec);
    a_close(fd);
    expect("null_path_bad_fd_ebadf", syscall4(SYS_utimensat, 99, 0, t, 0), -EBADF);
    expect("null_path_cwd_efault", syscall4(SYS_utimensat, AT_FDCWD, 0, t, 0), -EFAULT);
    struct a_timespec t2[2] = {{700, 0}, {800, 0}};
    expect("through_link", syscall4(SYS_utimensat, AT_FDCWD, D "/l", t2, 0), 0);
    a_stat_path(D "/f", &st);
    check("target_times_set", st.st_mtime_sec == 800, st.st_mtime_sec);
    struct a_timespec t3[2] = {{900, 0}, {1000, 0}};
    expect("link_nofollow", syscall4(SYS_utimensat, AT_FDCWD, D "/l", t3, AT_SYMLINK_NOFOLLOW), 0);
    a_lstat_path(D "/l", &st);
    check("link_times_set", st.st_mtime_sec == 1000, st.st_mtime_sec);
    a_stat_path(D "/f", &st);
    check("target_untouched", st.st_mtime_sec == 800, st.st_mtime_sec);
    expect("missing_enoent", syscall4(SYS_utimensat, AT_FDCWD, D "/none", t, 0), -ENOENT);
    expect("unknown_flag_einval", syscall4(SYS_utimensat, AT_FDCWD, D "/f", t, 0x1), -EINVAL);
    i64 dfd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("relative", syscall4(SYS_utimensat, dfd, "f", t, 0), 0);
    expect("bad_dirfd_ebadf", syscall4(SYS_utimensat, 99, "f", t, 0), -EBADF);
    a_close(dfd);
    struct a_timeval tv[2] = {{1500, 250000}, {1600, 999999}};
    expect("utimes_syscall", syscall2(SYS_utimes, D "/f", tv), 0);
    a_stat_path(D "/f", &st);
    check("utimes_microseconds", st.st_atime_sec == 1500 && st.st_atime_nsec == 250000000 &&
                                     st.st_mtime_sec == 1600 && st.st_mtime_nsec == 999999000, st.st_atime_nsec);
    struct a_timeval badtv[2] = {{1, 1000000}, {1, 0}};
    expect("utimes_bad_usec_einval", syscall2(SYS_utimes, D "/f", badtv), -EINVAL);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
