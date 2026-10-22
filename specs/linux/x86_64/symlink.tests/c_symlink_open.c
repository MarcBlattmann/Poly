/* symlink: what open and stat do with the links it makes. stat follows
 * (a dangling link is ENOENT), lstat does not. open with O_CREAT through
 * a dangling link creates its target (mode from open, less the umask);
 * O_CREAT|O_EXCL on any link is EEXIST; O_NOFOLLOW on a link is ELOOP,
 * O_PATH|O_NOFOLLOW opens the link itself; O_DIRECTORY through a link to
 * a directory works. */
#include "a_sys.h"

#define D "/tmp/c_symlink_open"

static char buf[16];

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);
    expect("link_file", syscall2(SYS_symlink, "f", D "/lf"), 0);
    expect("link_dir", syscall2(SYS_symlink, "sub", D "/ld"), 0);
    expect("link_dangling", syscall2(SYS_symlink, "made", D "/dang"), 0);
    expect("link_dangling_deep", syscall2(SYS_symlink, "nodir/made", D "/dang2"), 0);
    expect("link_root", syscall2(SYS_symlink, "/", D "/lroot"), 0);

    expect("stat_dangling_enoent", a_stat_path(D "/dang", &st), -ENOENT);
    expect("lstat_dangling_ok", a_lstat_path(D "/dang", &st), 0);
    expect("stat_root_link", a_stat_path(D "/lroot", &st), 0);
    expect("root_link_is_dir", st.st_mode & S_IFMT, S_IFDIR);
    expect("open_dangling_enoent", a_open(D "/dang", O_RDONLY, 0), -ENOENT);

    i64 fd = a_open(D "/dang", O_WRONLY | O_CREAT, 0666);
    check("creat_through_dangling", fd >= 0, fd);
    expect("write_through", a_write(fd, "hi", 2), 2);
    a_close(fd);
    expect("target_made", a_lstat_path(D "/made", &st), 0);
    expect("target_mode", st.st_mode, S_IFREG | 0644);
    expect("target_size", st.st_size, 2);
    expect("link_still_link", a_lstat_path(D "/dang", &st), 0);
    expect("link_still_link_mode", st.st_mode, S_IFLNK | 0777);
    expect("creat_missing_dir_enoent", a_open(D "/dang2", O_WRONLY | O_CREAT, 0644), -ENOENT);

    expect("excl_resolved_eexist", a_open(D "/dang", O_WRONLY | O_CREAT | O_EXCL, 0644), -EEXIST);
    expect("excl_dangling_eexist", a_open(D "/dang2", O_WRONLY | O_CREAT | O_EXCL, 0644), -EEXIST);
    expect("nofollow_eloop", a_open(D "/lf", O_RDONLY | O_NOFOLLOW, 0), -ELOOP);
    expect("nofollow_dangling_eloop", a_open(D "/dang2", O_RDONLY | O_NOFOLLOW, 0), -ELOOP);
    fd = a_open(D "/lf", O_PATH | O_NOFOLLOW, 0);
    check("path_nofollow_ok", fd >= 0, fd);
    expect("fstat_path_fd", syscall2(SYS_fstat, fd, &st), 0);
    expect("path_fd_is_link", st.st_mode, S_IFLNK | 0777);
    a_close(fd);
    fd = a_open(D "/ld", O_RDONLY | O_DIRECTORY, 0);
    check("directory_through_link", fd >= 0, fd);
    a_close(fd);
    expect("directory_through_filelink_enotdir", a_open(D "/lf", O_RDONLY | O_DIRECTORY, 0), -ENOTDIR);
    fd = a_open(D "/lf", O_RDONLY, 0);
    expect("read_through", a_read(fd, buf, sizeof buf), 4);
    a_close(fd);
    fd = a_open(D "/lf", O_WRONLY | O_TRUNC, 0);
    check("trunc_through", fd >= 0, fd);
    a_close(fd);
    expect("stat_after_trunc", a_stat_path(D "/f", &st), 0);
    expect("target_truncated", st.st_size, 0);
    expect("link_size_unchanged", a_lstat_path(D "/lf", &st), 0);
    expect("link_size", st.st_size, 1);

    const char *names[] = {D "/lf", D "/ld", D "/dang", D "/dang2", D "/lroot", D "/made", D "/f"};
    int ok = 1;
    for (int i = 0; i < 7; i++) ok &= a_unlink(names[i]) == 0;
    ok &= a_rmdir(D "/sub") == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
