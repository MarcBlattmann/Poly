/* rmdir in a sticky (S_ISVTX) directory: a caller that may write the
 * parent may remove a directory in it only if it owns that directory or
 * the parent; otherwise EPERM (also for a non-empty directory: EPERM
 * comes before ENOTEMPTY). Without write permission on the parent the
 * result is EACCES, before the sticky check. A missing name is ENOENT. In
 * a world-writable parent without the sticky bit anyone may remove any
 * empty directory. Root may remove anything in a sticky directory.
 * poly-diff: needs=root (directories owned by other users, setresuid)
 */
#include "a_sys.h"

#define D "/tmp/c_rmdir_sticky"
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119

#define U1 65533
#define U2 65534

static i64 rm(const char *p) { return syscall1(SYS_rmdir, p); }
static i64 own(const char *p, i64 uid) { return syscall3(SYS_chown, p, uid, uid); }

/* Run the removals named in `paths` as user `uid` in a child; results go
 * to sh[base..]. */
static void as_user(i64 uid, const char *const *paths, int n, int base) {
    volatile i32 *sh = a_shared();
    for (int i = 0; i < n; i++) sh[base + i] = 99;
    i64 pid = a_fork_sync();
    if (pid == 0) {
        if (syscall2(SYS_setgroups, 0, 0) != 0 || syscall3(SYS_setresgid, uid, uid, uid) != 0 ||
            syscall3(SYS_setresuid, uid, uid, uid) != 0)
            exit_group(2);
        for (int i = 0; i < n; i++) {
            if (paths[i][0] == '+') sh[base + i] = (i32)a_mkdir(paths[i] + 1, 0755);
            else sh[base + i] = (i32)rm(paths[i]);
        }
        exit_group(0);
    }
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    expect("child_exit_0", status, 0);
}

int test_main(void) {
    struct a_stat st;
    volatile i32 *sh = a_shared();
    a_block_sigchld();
    expect("setup", a_mkdir(D, 0755), 0);
    expect("mk_s", a_mkdir(D "/s", 0777), 0);
    expect("chmod_s_1777", syscall2(SYS_chmod, D "/s", 01777), 0);
    expect("mk_s_other", a_mkdir(D "/s/other", 0755), 0);
    expect("own_s_other_u2", own(D "/s/other", U2), 0);
    expect("mk_s_root", a_mkdir(D "/s/root", 0755), 0);
    expect("mk_s_full", a_mkdir(D "/s/full", 0777), 0);
    expect("mk_s_full_f", a_make_file(D "/s/full/f", "", 0644), 0);
    expect("own_s_full_u2", own(D "/s/full", U2), 0);
    expect("mk_t", a_mkdir(D "/t", 0777), 0);
    expect("chmod_t_1777", syscall2(SYS_chmod, D "/t", 01777), 0);
    expect("own_t_u1", own(D "/t", U1), 0);
    expect("mk_t_root", a_mkdir(D "/t/root", 0755), 0);
    expect("mk_w", a_mkdir(D "/w", 0777), 0);
    expect("chmod_w_0777", syscall2(SYS_chmod, D "/w", 0777), 0);
    expect("mk_w_root", a_mkdir(D "/w/root", 0755), 0);
    expect("mk_r", a_mkdir(D "/r", 0755), 0);
    expect("chmod_r_1755", syscall2(SYS_chmod, D "/r", 01755), 0);
    expect("mk_r_x", a_mkdir(D "/r/x", 0755), 0);
    expect("own_r_x_u1", own(D "/r/x", U1), 0);

    static const char *const u1_paths[] = {
        D "/s/other", D "/s/root", D "/s/full", D "/s/nope", "+" D "/s/mine", D "/s/mine",
        D "/t/root",  D "/w/root", D "/r/x",
    };
    as_user(U1, u1_paths, 9, 0);
    expect("u1_sticky_other_owner_eperm", sh[0], -EPERM);
    expect("u1_sticky_root_owned_eperm", sh[1], -EPERM);
    expect("u1_sticky_not_empty_eperm", sh[2], -EPERM);
    expect("u1_sticky_missing_enoent", sh[3], -ENOENT);
    expect("u1_mkdir_own", sh[4], 0);
    expect("u1_rmdir_own", sh[5], 0);
    expect("u1_owns_sticky_parent", sh[6], 0);
    expect("u1_not_sticky_world_writable", sh[7], 0);
    expect("u1_no_write_eacces_before_sticky", sh[8], -EACCES);
    expect("s_other_kept", a_stat_path(D "/s/other", &st), 0);
    expect("s_root_kept", a_stat_path(D "/s/root", &st), 0);
    expect("t_root_gone", a_lstat_path(D "/t/root", &st), -ENOENT);

    static const char *const u2_paths[] = {D "/s/other", D "/s/full"};
    as_user(U2, u2_paths, 2, 16);
    expect("u2_owner_removes", sh[16], 0);
    expect("u2_owner_not_empty_enotempty", sh[17], -ENOTEMPTY);

    expect("root_removes_in_sticky", rm(D "/s/root"), 0);
    expect("root_removes_r_x", rm(D "/r/x"), 0);
    expect("cleanup_full", a_unlink(D "/s/full/f") | rm(D "/s/full"), 0);
    expect("cleanup_dirs", rm(D "/s") | rm(D "/t") | rm(D "/w") | rm(D "/r"), 0);
    expect("cleanup_rmdir", rm(D), 0);
    return finish();
}
