/* creat(2) is open(path, O_CREAT|O_WRONLY|O_TRUNC, mode): it creates the
 * file (mode & ~umask) or truncates an existing one, and returns the
 * lowest free descriptor, open for writing only, offset 0, not
 * close-on-exec. A dangling symbolic link is followed: the target is
 * created. */
#include "a_sys.h"

#define D "/tmp/lb_creat_basic"

static i64 creat_(const char *p, i64 mode) { return syscall2(85, p, mode); }

int test_main(void) {
    struct a_stat st;
    char c[16];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);

    i64 fd = creat_(D "/new", 0666);
    expect("returns_lowest_fd", fd, 3);
    expect("flags_wronly", syscall2(SYS_fcntl, fd, F_GETFL) & (O_ACCMODE | O_APPEND | O_NONBLOCK), O_WRONLY);
    expect("not_cloexec", syscall2(SYS_fcntl, fd, F_GETFD), 0);
    expect("stat_new", a_stat_path(D "/new", &st), 0);
    expect("mode_umask_applied", st.st_mode, S_IFREG | 0644);
    expect("size_0", st.st_size, 0);
    expect("uid", st.st_uid, syscall0(SYS_geteuid));
    expect("gid", st.st_gid, syscall0(SYS_getegid));
    expect("write", a_write(fd, "hello", 5), 5);
    expect("read_ebadf", a_read(fd, c, 5), -EBADF);
    expect("close", a_close(fd), 0);

    /* an existing file is truncated, its mode and inode kept */
    i64 ino = (i64)st.st_ino;
    expect("chmod_0600", syscall2(SYS_chmod, D "/new", 0600), 0);
    fd = creat_(D "/new", 0777);
    expect("again_fd_3", fd, 3);
    expect("stat_again", a_stat_path(D "/new", &st), 0);
    expect("truncated", st.st_size, 0);
    expect("mode_kept", st.st_mode, S_IFREG | 0600);
    expect("same_inode", (i64)st.st_ino, ino);
    expect("close2", a_close(fd), 0);

    /* mode bits: umask, set-id and sticky, type bits ignored */
    fd = creat_(D "/m1", 07777);
    expect("m1_mode", a_stat_path(D "/m1", &st) == 0 ? (i64)st.st_mode : -1, S_IFREG | 07755);
    a_close(fd);
    fd = creat_(D "/m2", 0);
    expect("m2_mode", a_stat_path(D "/m2", &st) == 0 ? (i64)st.st_mode : -1, S_IFREG);
    /* a mode-0 file was still opened for writing by its creator */
    expect("m2_write", a_write(fd, "x", 1), 1);
    a_close(fd);
    fd = creat_(D "/m3", S_IFDIR | 0640);
    expect("m3_mode", a_stat_path(D "/m3", &st) == 0 ? (i64)st.st_mode : -1, S_IFREG | 0640);
    a_close(fd);

    /* symbolic links: an existing target is truncated, a dangling link
     * creates its target */
    expect("setup_target", a_make_file(D "/target", "data", 0644), 0);
    expect("setup_l", syscall2(SYS_symlink, "target", D "/l"), 0);
    fd = creat_(D "/l", 0644);
    check("via_link", fd >= 0, fd);
    a_close(fd);
    expect("target_truncated", a_stat_path(D "/target", &st) == 0 && st.st_size == 0, 1);
    expect("setup_dangling", syscall2(SYS_symlink, "made_by_creat", D "/dangling"), 0);
    fd = creat_(D "/dangling", 0640);
    check("via_dangling", fd >= 0, fd);
    a_close(fd);
    expect("target_created", a_lstat_path(D "/made_by_creat", &st) == 0 && st.st_mode == (S_IFREG | 0640), 1);

    /* lowest free descriptor */
    expect("close_0", a_close(0), 0);
    fd = creat_(D "/low", 0644);
    expect("fd_0_reused", fd, 0);
    a_close(fd);

    /* relative paths */
    expect("chdir", syscall1(SYS_chdir, D), 0);
    fd = creat_("rel", 0644);
    check("relative", fd >= 0, fd);
    a_close(fd);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("rel_exists", a_stat_path(D "/rel", &st), 0);

    static const char *rm[] = {D "/rel", D "/low", D "/made_by_creat", D "/dangling", D "/l", D "/target",
                               D "/m1", D "/m2", D "/m3", D "/new"};
    for (unsigned i = 0; i < sizeof rm / sizeof rm[0]; i++) expect("cleanup_unlink", a_unlink(rm[i]), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
