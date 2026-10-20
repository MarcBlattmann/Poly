/* chdir: AT_FDCWD in the *at calls means the directory chdir chose;
 * descriptors opened before the chdir keep their own directory. */
#include "a_sys.h"

#define D "/tmp/a_chdir_atfdcwd"

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_a", a_mkdir(D "/a", 0700), 0);
    expect("setup_b", a_mkdir(D "/b", 0700), 0);
    expect("setup_fa", a_make_file(D "/a/f", "A", 0644), 0);
    expect("setup_fb", a_make_file(D "/b/f", "BB", 0644), 0);
    expect("chdir_a", syscall1(SYS_chdir, D "/a"), 0);
    i64 da = a_open(".", O_RDONLY | O_DIRECTORY, 0);
    expect("stat_in_a", syscall4(SYS_newfstatat, AT_FDCWD, "f", &st, 0) == 0 ? st.st_size : -1, 1);
    expect("chdir_b", syscall1(SYS_chdir, "../b"), 0);
    expect("stat_in_b", syscall4(SYS_newfstatat, AT_FDCWD, "f", &st, 0) == 0 ? st.st_size : -1, 2);
    expect("old_dirfd_still_a", syscall4(SYS_newfstatat, da, "f", &st, 0) == 0 ? st.st_size : -1, 1);
    expect("mkdirat_cwd", syscall3(SYS_mkdirat, AT_FDCWD, "made", 0700), 0);
    expect("made_in_b", a_stat_path(D "/b/made", &st), 0);
    expect("unlinkat_cwd", syscall3(SYS_unlinkat, AT_FDCWD, "made", AT_REMOVEDIR), 0);
    expect("rename_cwd_relative", syscall5(SYS_renameat2, AT_FDCWD, "f", da, "g", 0), 0);
    expect("moved_from_b_to_a", a_stat_path(D "/a/g", &st) == 0 ? st.st_size : -1, 2);
    a_close(da);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("cleanup_g", a_unlink(D "/a/g"), 0);
    expect("cleanup_fa", a_unlink(D "/a/f"), 0);
    expect("cleanup_a", a_rmdir(D "/a"), 0);
    expect("cleanup_b", a_rmdir(D "/b"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
