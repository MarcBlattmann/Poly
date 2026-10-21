/* access checks with the real uid and gid, not the effective ones: with real
 * uid 0 and effective uid 65534 a root-only file passes; with real uid 65534
 * and effective uid 0 it fails (EACCES), and so does root's X_OK shortcut;
 * the group bits apply when the real gid matches the file's group.
 * poly-diff: needs=root (only root may switch real and effective ids)
 */
#include "a_sys.h"

#define D "/tmp/a_access_realid"
#define SYS_setresuid 117
#define SYS_setresgid 119

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_secret", a_make_file(D "/secret", "", 0600), 0);
    expect("setup_grp", a_make_file(D "/grp", "", 0600), 0);
    expect("chown_grp", syscall3(SYS_chown, D "/grp", 0, 4321), 0);
    expect("chmod_grp", syscall2(SYS_chmod, D "/grp", 0060), 0);
    expect("setup_other", a_make_file(D "/other", "", 0600), 0);
    expect("chmod_other", syscall2(SYS_chmod, D "/other", 0004), 0);

    /* Real 0, effective 65534. */
    expect("set_e_nobody", syscall3(SYS_setresuid, 0, 65534, 0), 0);
    expect("geteuid_nobody", syscall0(SYS_geteuid), 65534);
    expect("real_root_r_secret", syscall2(SYS_access, D "/secret", R_OK | W_OK), 0);
    expect("open_as_effective_eacces", a_open(D "/secret", O_RDONLY, 0), -EACCES);
    expect("back_root_1", syscall3(SYS_setresuid, 0, 0, 0), 0);

    /* Real 65534, effective 0. */
    expect("set_r_nobody", syscall3(SYS_setresuid, 65534, 0, 0), 0);
    expect("real_nobody_r_secret", syscall2(SYS_access, D "/secret", R_OK), -EACCES);
    expect("real_nobody_f_secret", syscall2(SYS_access, D "/secret", F_OK), 0);
    expect("real_nobody_x_dir", syscall2(SYS_access, D, X_OK), 0);
    expect("real_nobody_w_dir", syscall2(SYS_access, D, W_OK), -EACCES);
    expect("real_nobody_other_r", syscall2(SYS_access, D "/other", R_OK), 0);
    expect("real_nobody_other_w", syscall2(SYS_access, D "/other", W_OK), -EACCES);
    i64 fd = a_open(D "/secret", O_RDONLY, 0);
    check("open_as_effective_ok", fd >= 0, fd);
    if (fd >= 0) a_close(fd);
    expect("real_nobody_grp_r_wrong_gid", syscall2(SYS_access, D "/grp", R_OK), -EACCES);
    expect("set_rgid_4321", syscall3(SYS_setresgid, 4321, 0, 0), 0);
    expect("real_gid_grp_rw", syscall2(SYS_access, D "/grp", R_OK | W_OK), 0);
    expect("real_gid_grp_x", syscall2(SYS_access, D "/grp", X_OK), -EACCES);
    expect("set_egid_only", syscall3(SYS_setresgid, 0, 4321, 0), 0);
    expect("effective_gid_ignored", syscall2(SYS_access, D "/grp", R_OK), -EACCES);
    expect("back_gid", syscall3(SYS_setresgid, 0, 0, 0), 0);
    expect("back_root_2", syscall3(SYS_setresuid, 0, 0, 0), 0);
    expect("root_again_r_secret", syscall2(SYS_access, D "/secret", R_OK), 0);

    int ok = a_unlink(D "/secret") == 0 && a_unlink(D "/grp") == 0 && a_unlink(D "/other") == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
