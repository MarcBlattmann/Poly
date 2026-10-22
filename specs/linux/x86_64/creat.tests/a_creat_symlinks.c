/* creat(2) and symbolic links: a link to a file is followed (the target is
 * truncated, the link stays), a dangling link creates its target, a link
 * to a directory gives EISDIR, loops give ELOOP. */
#include "a_sys.h"

#define SYS_creat 85
#define D "/tmp/a_creat_symlinks"

static i64 cr(const char *p, i64 mode) { return syscall2(SYS_creat, p, mode); }
static i64 sl(const char *target, const char *path) { return syscall3(SYS_symlinkat, target, AT_FDCWD, path); }

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0700), 0);
    expect("setup_target", a_make_file(D "/t", "contents", 0640), 0);
    expect("setup_link_file", sl("t", D "/lf"), 0);
    expect("setup_link_dangling", sl("sub/made", D "/ld"), 0);
    expect("setup_link_dir", sl("sub", D "/ldir"), 0);
    expect("setup_link_nodir", sl("nodir/x", D "/lnd"), 0);
    expect("setup_loop_a", sl("lb", D "/la"), 0);
    expect("setup_loop_b", sl("la", D "/lb"), 0);
    expect("setup_self", sl("self", D "/self"), 0);

    i64 fd = cr(D "/lf", 0600);
    expect("through_link_fd", fd, 3);
    expect("through_link_is_target", syscall2(SYS_fstat, fd, &st) == 0 ? (i64)st.st_mode : -1, S_IFREG | 0640);
    a_close(fd);
    expect("target_truncated", a_stat_path(D "/t", &st) == 0 ? (i64)st.st_size : -1, 0);
    expect("link_still_link", a_lstat_path(D "/lf", &st) == 0 ? (i64)(st.st_mode & S_IFMT) : -1, S_IFLNK);

    /* A dangling link: its target is created relative to the link's dir. */
    fd = cr(D "/ld", 0640);
    expect("dangling_fd", fd, 3);
    a_close(fd);
    expect("dangling_target_made", a_stat_path(D "/sub/made", &st) == 0 ? (i64)st.st_mode : -1, S_IFREG | 0640);
    expect("dangling_link_stays", a_lstat_path(D "/ld", &st) == 0 ? (i64)(st.st_mode & S_IFMT) : -1, S_IFLNK);

    expect("link_to_dir_eisdir", cr(D "/ldir", 0644), -EISDIR);
    fd = cr(D "/ldir/inside", 0644);
    expect("through_dir_link_component", fd, 3);
    a_close(fd);
    expect("inside_made", a_stat_path(D "/sub/inside", &st), 0);

    expect("dangling_missing_dir_enoent", cr(D "/lnd", 0644), -ENOENT);
    expect("loop_eloop", cr(D "/la", 0644), -ELOOP);
    expect("self_eloop", cr(D "/self", 0644), -ELOOP);
    expect("loop_component_eloop", cr(D "/la/x", 0644), -ELOOP);

    const char *names[] = {D "/sub/made", D "/sub/inside", D "/t", D "/lf", D "/ld", D "/ldir", D "/lnd", D "/la", D "/lb", D "/self"};
    for (int i = 0; i < 10; i++) expect("cleanup_unlink", a_unlink(names[i]), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
