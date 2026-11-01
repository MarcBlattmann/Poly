/* unlink(2) removes one name: the file lives on while other names or open
 * descriptors remain (st_nlink counts the names, its ctime changes); a
 * symbolic link is removed itself, never its target; FIFOs, sockets and
 * dangling links are removed like files; the parent's mtime/ctime change. */
#include "a_sys.h"

#define D "/tmp/lb_unlink_basic"
#define SYS_mknodat 259

static i64 unlink_(const char *p) { return syscall1(SYS_unlink, p); }
static i64 nlink_of(const char *p) {
    struct a_stat st;
    if (a_lstat_path(p, &st)) return -1;
    return (i64)st.st_nlink;
}

int test_main(void) {
    struct a_stat st, st2, f0;
    char c[16];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "content", 0644), 0);
    expect("link_h1", syscall2(SYS_link, D "/f", D "/h1"), 0);
    expect("link_h2", syscall2(SYS_link, D "/f", D "/h2"), 0);
    expect("nlink_3", nlink_of(D "/f"), 3);

    struct a_timespec ts[2] = {{1000000000, 0}, {1000000000, 0}};
    expect("set_file_times", syscall4(SYS_utimensat, AT_FDCWD, D "/f", ts, 0), 0);
    expect("set_dir_times", syscall4(SYS_utimensat, AT_FDCWD, D, ts, 0), 0);
    expect("stat_f0", a_stat_path(D "/f", &f0), 0);
    expect("stat_d0", a_stat_path(D, &st2), 0);
    struct a_timespec pause = {0, 20000000};
    syscall2(SYS_nanosleep, &pause, 0);
    expect("unlink_f", unlink_(D "/f"), 0);
    expect("f_gone", a_lstat_path(D "/f", &st), -ENOENT);
    expect("h1_nlink_2", nlink_of(D "/h1"), 2);
    expect("stat_h1", a_stat_path(D "/h1", &st), 0);
    check("file_ctime_changed",
          st.st_ctime_sec > f0.st_ctime_sec || (st.st_ctime_sec == f0.st_ctime_sec && st.st_ctime_nsec > f0.st_ctime_nsec), 0);
    expect("file_mtime_kept", st.st_mtime_sec, 1000000000);
    expect("stat_d1", a_stat_path(D, &st), 0);
    check("dir_mtime_changed", st.st_mtime_sec > 1000000000, 0);
    check("dir_ctime_changed",
          st.st_ctime_sec > st2.st_ctime_sec || (st.st_ctime_sec == st2.st_ctime_sec && st.st_ctime_nsec > st2.st_ctime_nsec), 0);
    expect("dir_atime_kept", st.st_atime_sec, 1000000000);
    expect("unlink_f_again", unlink_(D "/f"), -ENOENT);

    /* an open descriptor keeps the last name's file alive */
    i64 fd = a_open(D "/h1", O_RDWR, 0);
    check("open_h1", fd >= 0, fd);
    expect("unlink_h1", unlink_(D "/h1"), 0);
    expect("unlink_h2", unlink_(D "/h2"), 0);
    expect("fstat_open", syscall2(SYS_fstat, fd, &st), 0);
    expect("open_nlink_0", st.st_nlink, 0);
    expect("open_size", st.st_size, 7);
    expect("pread", syscall4(SYS_pread64, fd, c, 16, 0), 7);
    expect("contents", mem_eq(c, "content", 7), 1);
    expect("write_after_unlink", a_write(fd, "!", 1), 1);
    /* the name is free: a new file is a different inode */
    expect("new_same_name", a_make_file(D "/h1", "new", 0644), 0);
    expect("stat_new", a_stat_path(D "/h1", &st2), 0);
    check("different_inode", st2.st_ino != st.st_ino, 0);
    expect("new_size", st2.st_size, 3);
    expect("close", a_close(fd), 0);
    expect("unlink_new", unlink_(D "/h1"), 0);

    /* symbolic links: the link goes, the target stays */
    expect("setup_target", a_make_file(D "/target", "t", 0644), 0);
    expect("setup_link", syscall2(SYS_symlink, "target", D "/l"), 0);
    expect("unlink_link", unlink_(D "/l"), 0);
    expect("link_gone", a_lstat_path(D "/l", &st), -ENOENT);
    expect("target_kept", a_stat_path(D "/target", &st), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "nothing", D "/dangling"), 0);
    expect("unlink_dangling", unlink_(D "/dangling"), 0);
    expect("setup_dirlink", syscall2(SYS_symlink, "sub", D "/dirlink"), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("unlink_dirlink", unlink_(D "/dirlink"), 0);
    expect("sub_kept", a_stat_path(D "/sub", &st), 0);
    expect("setup_loop", syscall2(SYS_symlink, "loop", D "/loop"), 0);
    expect("unlink_loop", unlink_(D "/loop"), 0);

    /* other kinds */
    expect("mkfifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", S_IFIFO | 0644, 0), 0);
    expect("unlink_fifo", unlink_(D "/fifo"), 0);
    expect("file_mode_0", a_make_file(D "/m0", "x", 0), 0);
    expect("unlink_mode_0", unlink_(D "/m0"), 0);

    /* relative paths */
    expect("setup_rel", a_make_file(D "/sub/r", "x", 0644), 0);
    expect("chdir", syscall1(SYS_chdir, D "/sub"), 0);
    expect("relative", unlink_("r"), 0);
    expect("relative_dotdot", unlink_("../target"), 0);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);

    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
