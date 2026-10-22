/* unlink with a trailing slash: the last name is not followed even then,
 * so any existing non-directory, a symbolic link of any kind included, is
 * ENOTDIR; a missing name ENOENT, a directory EISDIR; nothing is
 * removed. */
#include "a_sys.h"

#define D "/tmp/c_unlink_trailing_slash"

static i64 u(const char *p) { return syscall1(SYS_unlink, p); }

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_dir", a_mkdir(D "/d", 0755), 0);
    expect("setup_lf", syscall2(SYS_symlink, "f", D "/lf"), 0);
    expect("setup_ld", syscall2(SYS_symlink, "d", D "/ld"), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "nowhere", D "/ln"), 0);
    expect("file_slash_enotdir", u(D "/f/"), -ENOTDIR);
    expect("file_slash_dot_enotdir", u(D "/f/."), -ENOTDIR);
    expect("missing_slash_enoent", u(D "/nope/"), -ENOENT);
    expect("link_to_file_slash_enotdir", u(D "/lf/"), -ENOTDIR);
    expect("link_to_dir_slash_enotdir", u(D "/ld/"), -ENOTDIR);
    expect("dangling_slash_enotdir", u(D "/ln/"), -ENOTDIR);
    expect("dir_slash_eisdir", u(D "/d/"), -EISDIR);
    expect("dir_slashes_eisdir", u(D "/d//"), -EISDIR);
    expect("file_kept", a_stat_path(D "/f", &st), 0);
    expect("lf_kept", a_lstat_path(D "/lf", &st), 0);
    expect("ld_kept", a_lstat_path(D "/ld", &st), 0);
    expect("ln_kept", a_lstat_path(D "/ln", &st), 0);
    expect("double_slash_inside_ok", u(D "//f"), 0);
    expect("unlink_lf", u(D "/lf"), 0);
    expect("unlink_ld", u(D "/ld"), 0);
    expect("unlink_ln", u(D "/ln"), 0);
    expect("cleanup_d", syscall1(SYS_rmdir, D "/d"), 0);
    expect("cleanup_dir", syscall1(SYS_rmdir, D), 0);
    return finish();
}
