/* mkdirat: creates an empty directory with the requested mode; existing
 * names, missing parents and files in the way fail. */
#include "a_sys.h"

#define D "/tmp/a_mkdirat_basic"

int test_main(void) {
    struct a_stat st;
    expect("mkdir", a_mkdir(D, 0750), 0);
    expect("stat", a_stat_path(D, &st), 0);
    expect("mode_dir_0750", st.st_mode, S_IFDIR | 0750);
    expect("nlink_2", st.st_nlink, 2);
    expect("owner_is_caller", st.st_uid, syscall0(SYS_getuid));
    expect("mkdir_again_eexist", a_mkdir(D, 0750), -EEXIST);
    expect("mkdir_trailing_slash_eexist", a_mkdir(D "/", 0750), -EEXIST);
    expect("missing_parent_enoent", a_mkdir(D "/a/b", 0700), -ENOENT);
    expect("file", a_make_file(D "/f", "", 0644), 0);
    expect("over_file_eexist", a_mkdir(D "/f", 0700), -EEXIST);
    expect("under_file_enotdir", a_mkdir(D "/f/sub", 0700), -ENOTDIR);
    expect("new_with_trailing_slash", a_mkdir(D "/s/", 0700), 0);
    expect("trailing_slash_made_dir", a_stat_path(D "/s", &st) == 0 ? (i64)(st.st_mode & S_IFMT) : -1, S_IFDIR);
    expect("dot_eexist", a_mkdir(D "/.", 0700), -EEXIST);
    expect("dotdot_eexist", a_mkdir(D "/..", 0700), -EEXIST);
    expect("root_eexist", a_mkdir("/", 0700), -EEXIST);
    expect("empty_enoent", a_mkdir("", 0700), -ENOENT);
    expect("bad_path_efault", a_mkdir((const char *)8, 0700), -EFAULT);
    expect("symlink", syscall3(SYS_symlinkat, "target", AT_FDCWD, D "/l"), 0);
    expect("over_dangling_symlink_eexist", a_mkdir(D "/l", 0700), -EEXIST);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_s", a_rmdir(D "/s"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
