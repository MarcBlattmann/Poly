/* access follows symbolic links, at the end of the path and on the way: the
 * mode checked is the target's; a dangling link is ENOENT; a loop is ELOOP;
 * the link's own mode (0777) is never used. */
#include "a_sys.h"

#define D "/tmp/a_access_links"

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/sub/f", "", 0644), 0);
    expect("setup_exec", a_make_file(D "/e", "", 0755), 0);
    expect("setup_ro", a_make_file(D "/ro", "", 0400), 0);
    expect("ln_f", syscall2(SYS_symlink, "sub/f", D "/lf"), 0);
    expect("ln_e", syscall2(SYS_symlink, D "/e", D "/le"), 0);
    expect("ln_ro", syscall2(SYS_symlink, "ro", D "/lro"), 0);
    expect("ln_dir", syscall2(SYS_symlink, "sub", D "/ld"), 0);
    expect("ln_chain", syscall2(SYS_symlink, "lf", D "/lchain"), 0);
    expect("ln_dangling", syscall2(SYS_symlink, "gone", D "/dang"), 0);
    expect("ln_loop_a", syscall2(SYS_symlink, "loopb", D "/loopa"), 0);
    expect("ln_loop_b", syscall2(SYS_symlink, "loopa", D "/loopb"), 0);
    expect("ln_self", syscall2(SYS_symlink, "self", D "/self"), 0);

    expect("link_f_ok", syscall2(SYS_access, D "/lf", F_OK), 0);
    expect("link_rw", syscall2(SYS_access, D "/lf", R_OK | W_OK), 0);
    expect("link_target_no_x", syscall2(SYS_access, D "/lf", X_OK), -EACCES);
    expect("link_target_x", syscall2(SYS_access, D "/le", X_OK), 0);
    expect("link_target_ro_w", syscall2(SYS_access, D "/lro", W_OK), root ? 0 : -EACCES);
    expect("link_dir", syscall2(SYS_access, D "/ld", X_OK), 0);
    expect("link_dir_slash", syscall2(SYS_access, D "/ld/", F_OK), 0);
    expect("through_link_dir", syscall2(SYS_access, D "/ld/f", R_OK), 0);
    expect("link_chain", syscall2(SYS_access, D "/lchain", R_OK), 0);
    expect("dangling_enoent", syscall2(SYS_access, D "/dang", F_OK), -ENOENT);
    expect("dangling_slash_enoent", syscall2(SYS_access, D "/dang/", F_OK), -ENOENT);
    expect("loop_eloop", syscall2(SYS_access, D "/loopa", F_OK), -ELOOP);
    expect("self_eloop", syscall2(SYS_access, D "/self", F_OK), -ELOOP);
    expect("loop_mid_eloop", syscall2(SYS_access, D "/loopa/x", F_OK), -ELOOP);
    expect("link_file_slash_enotdir", syscall2(SYS_access, D "/lf/", F_OK), -ENOTDIR);

    const char *names[] = {D "/lf", D "/le", D "/lro", D "/ld", D "/lchain", D "/dang",
                           D "/loopa", D "/loopb", D "/self", D "/e", D "/ro", D "/sub/f"};
    int ok = 1;
    for (int i = 0; i < 12; i++) ok &= a_unlink(names[i]) == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
