/* utimess acts on the inode: the new times are seen through a hard link and
 * through descriptors that were open before the call. */
#include "a_sys.h"

#define D "/tmp/c_utimes_links_fds"

/* sys/time.h: struct timeval { time_t tv_sec; suseconds_t tv_usec; } */
struct timeval_k {
    i64 tv_sec, tv_usec;
};

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/a", "abc", 0644), 0);
    expect("setup_hardlink", syscall5(SYS_linkat, AT_FDCWD, D "/a", AT_FDCWD, D "/b", 0), 0);
    i64 fd = a_open(D "/a", O_RDWR, 0);
    check("setup_open", fd >= 0, fd);
    struct timeval_k t[2] = {{31337, 0}, {7331, 0}};
    expect("via_name_b", syscall2(SYS_utimes, D "/b", t), 0);
    a_stat_path(D "/a", &st);
    check("seen_via_a", st.st_atime_sec == 31337 && st.st_mtime_sec == 7331, (i64)st.st_mtime_sec);
    expect("fstat", syscall2(SYS_fstat, fd, &st), 0);
    check("seen_via_fd", st.st_atime_sec == 31337 && st.st_mtime_sec == 7331, (i64)st.st_mtime_sec);
    check("nlink_kept", st.st_nlink == 2, (i64)st.st_nlink);
    /* A write through the descriptor afterwards moves mtime to now. */
    expect("write", a_write(fd, "z", 1), 1);
    syscall2(SYS_fstat, fd, &st);
    check("write_moves_mtime", st.st_mtime_sec > 7331, (i64)st.st_mtime_sec);
    check("write_keeps_atime", st.st_atime_sec == 31337, (i64)st.st_atime_sec);
    expect("close", a_close(fd), 0);
    /* Unlinked but still open: a path is needed, so the other name works. */
    expect("unlink_a", a_unlink(D "/a"), 0);
    expect("gone_name", syscall2(SYS_utimes, D "/a", t), -ENOENT);
    expect("remaining_name", syscall2(SYS_utimes, D "/b", t), 0);
    a_stat_path(D "/b", &st);
    check("remaining_values", st.st_mtime_sec == 7331, (i64)st.st_mtime_sec);
    expect("cleanup_b", a_unlink(D "/b"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
