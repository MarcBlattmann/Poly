/* symlink: path errors. An empty target or an empty linkpath is ENOENT;
 * a missing parent is ENOENT; a regular file used as a directory is
 * ENOTDIR (also "file/."); a new name with a trailing slash is ENOENT;
 * an unreadable target or linkpath pointer is EFAULT. Nothing is created
 * by a failed call. The target is checked before the linkpath. */
#include "a_sys.h"

#define D "/tmp/c_symlink_errors"

static char big[4200];

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);

    expect("empty_target_enoent", syscall2(SYS_symlink, "", D "/e"), -ENOENT);
    expect("empty_target_not_made", a_lstat_path(D "/e", &st), -ENOENT);
    expect("empty_linkpath_enoent", syscall2(SYS_symlink, "x", ""), -ENOENT);
    expect("missing_parent_enoent", syscall2(SYS_symlink, "x", D "/no/l"), -ENOENT);
    expect("missing_parents_enoent", syscall2(SYS_symlink, "x", D "/no/no2/l"), -ENOENT);
    expect("missing_then_dotdot_enoent", syscall2(SYS_symlink, "x", D "/no/../l"), -ENOENT);
    expect("not_made_via_dotdot", a_lstat_path(D "/l", &st), -ENOENT);
    expect("file_as_dir_enotdir", syscall2(SYS_symlink, "x", D "/f/l"), -ENOTDIR);
    expect("file_dot_enotdir", syscall2(SYS_symlink, "x", D "/f/."), -ENOTDIR);
    expect("file_as_dir_deep_enotdir", syscall2(SYS_symlink, "x", D "/f/a/b"), -ENOTDIR);
    expect("new_name_slash_enoent", syscall2(SYS_symlink, "x", D "/new/"), -ENOENT);
    expect("new_name_slash_not_made", a_lstat_path(D "/new", &st), -ENOENT);
    expect("bad_target_efault", syscall2(SYS_symlink, (const char *)8, D "/b"), -EFAULT);
    expect("bad_linkpath_efault", syscall2(SYS_symlink, "x", (const char *)8), -EFAULT);
    expect("bad_target_not_made", a_lstat_path(D "/b", &st), -ENOENT);

    /* Precedence: the whole target first, then the linkpath. */
    for (int i = 0; i < 4096; i++) big[i] = 'a';
    big[4096] = 0;
    expect("empty_target_before_bad_linkpath", syscall2(SYS_symlink, "", (const char *)8), -ENOENT);
    expect("long_target_before_bad_linkpath", syscall2(SYS_symlink, big, (const char *)8), -ENAMETOOLONG);
    expect("bad_target_before_empty_linkpath", syscall2(SYS_symlink, (const char *)8, ""), -EFAULT);
    expect("empty_target_before_eexist", syscall2(SYS_symlink, "", D "/f"), -ENOENT);
    expect("empty_target_before_enotdir", syscall2(SYS_symlink, "", D "/f/x"), -ENOENT);
    expect("long_target_before_missing_dir", syscall2(SYS_symlink, big, D "/no/x"), -ENAMETOOLONG);
    expect("missing_dir_before_eexist_name", syscall2(SYS_symlink, "x", D "/no/f"), -ENOENT);

    expect("cleanup_file", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
