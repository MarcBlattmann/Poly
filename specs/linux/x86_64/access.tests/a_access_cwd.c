/* access: a relative path starts at the working directory; after a chdir
 * the same name means another file; "." is the working directory. */
#include "a_sys.h"

#define D "/tmp/a_access_cwd"

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_a", a_mkdir(D "/a", 0755), 0);
    expect("setup_b", a_mkdir(D "/b", 0755), 0);
    expect("setup_file_a", a_make_file(D "/a/only_a", "", 0755), 0);
    expect("setup_file_b", a_make_file(D "/b/only_b", "", 0644), 0);
    expect("start_in_root_relative", syscall2(SYS_access, "tmp/a_access_cwd/a/only_a", F_OK), 0);
    expect("chdir_a", syscall1(SYS_chdir, D "/a"), 0);
    expect("rel_a", syscall2(SYS_access, "only_a", X_OK), 0);
    expect("rel_b_missing", syscall2(SYS_access, "only_b", F_OK), -ENOENT);
    expect("rel_dot", syscall2(SYS_access, ".", R_OK | X_OK), 0);
    expect("rel_dot_slash", syscall2(SYS_access, "./only_a", R_OK), 0);
    expect("rel_up", syscall2(SYS_access, "../b/only_b", R_OK), 0);
    expect("rel_up_no_x", syscall2(SYS_access, "../b/only_b", X_OK), -EACCES);
    expect("chdir_b", syscall1(SYS_chdir, D "/b"), 0);
    expect("rel_b", syscall2(SYS_access, "only_b", R_OK), 0);
    expect("rel_a_missing", syscall2(SYS_access, "only_a", F_OK), -ENOENT);
    expect("abs_ignores_cwd", syscall2(SYS_access, D "/a/only_a", F_OK), 0);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    int ok = a_unlink(D "/a/only_a") == 0 && a_unlink(D "/b/only_b") == 0;
    ok &= a_rmdir(D "/a") == 0 && a_rmdir(D "/b") == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
