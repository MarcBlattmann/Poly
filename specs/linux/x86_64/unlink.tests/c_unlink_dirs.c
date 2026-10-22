/* unlink on directories: EISDIR, whatever the spelling ("dir", "dir/",
 * "dir/.", "dir/..", "/", "."), and the directory is left in place. */
#include "a_sys.h"

#define D "/tmp/c_unlink_dirs"

static i64 u(const char *p) { return syscall1(SYS_unlink, p); }

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_empty", a_mkdir(D "/empty", 0755), 0);
    expect("setup_full", a_mkdir(D "/full", 0755), 0);
    expect("setup_inner", a_make_file(D "/full/f", "", 0644), 0);
    expect("empty_dir_eisdir", u(D "/empty"), -EISDIR);
    expect("full_dir_eisdir", u(D "/full"), -EISDIR);
    expect("trailing_slash_eisdir", u(D "/empty/"), -EISDIR);
    expect("dot_eisdir", u(D "/empty/."), -EISDIR);
    expect("dotdot_eisdir", u(D "/empty/.."), -EISDIR);
    expect("root_eisdir", u("/"), -EISDIR);
    expect("cwd_dot_eisdir", u("."), -EISDIR);
    expect("cwd_dotdot_eisdir", u(".."), -EISDIR);
    expect("empty_still_there", a_stat_path(D "/empty", &st), 0);
    expect("empty_is_dir", st.st_mode & S_IFMT, S_IFDIR);
    expect("full_still_there", a_stat_path(D "/full/f", &st), 0);
    expect("parent_nlink_4", a_stat_path(D, &st) == 0 ? (i64)st.st_nlink : -1, 4);
    expect("cleanup_inner", u(D "/full/f"), 0);
    expect("cleanup_full", syscall1(SYS_rmdir, D "/full"), 0);
    expect("cleanup_empty", syscall1(SYS_rmdir, D "/empty"), 0);
    expect("cleanup_dir", syscall1(SYS_rmdir, D), 0);
    return finish();
}
