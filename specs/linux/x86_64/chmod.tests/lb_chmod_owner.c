/* chmod(2) by a user other than root: only the owner may change the mode
 * (EPERM otherwise, even for the same mode and with write permission on
 * the file); set-group-ID is silently dropped when the caller is not in the
 * file's group; root keeps it. A read-only mount answers EROFS before
 * ownership is checked.
 * poly-diff: needs=root (chowns files to other users, changes uid, mounts)
 */
#include "a_sys.h"

#define D "/tmp/lb_chmod_owner"
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_mount 165
#define SYS_umount2 166
#define MS_RDONLY 1
#define MS_REMOUNT 32

static i64 chmod_(const char *p, i64 mode) { return syscall2(SYS_chmod, p, mode); }
static i64 chown_(const char *p, i64 u, i64 g) { return syscall3(SYS_chown, p, u, g); }
static u32 mode_of(const char *p) {
    struct a_stat st;
    if (a_stat_path(p, &st)) return 0xffffffff;
    return st.st_mode;
}

static int child_user(void) {
    u32 groups[1] = {3000};
    expect("c_setgroups", syscall2(SYS_setgroups, 1, groups), 0);
    expect("c_setresgid", syscall3(SYS_setresgid, 1000, 1000, 1000), 0);
    expect("c_setresuid", syscall3(SYS_setresuid, 1000, 1000, 1000), 0);
    /* not the owner: EPERM, even with rw for others, even for the same mode */
    expect("c_other_owner_eperm", chmod_(D "/theirs", 0600), -EPERM);
    expect("c_other_same_mode_eperm", chmod_(D "/theirs", 0666), -EPERM);
    expect("c_theirs_unchanged", mode_of(D "/theirs"), S_IFREG | 0666);
    expect("c_group_member_eperm", chmod_(D "/grpfile", 0600), -EPERM);
    /* owner, group 1000 (own group): set-group-ID kept */
    expect("c_own_sgid", chmod_(D "/mine", 02755), 0);
    expect("c_own_sgid_kept", mode_of(D "/mine"), S_IFREG | 02755);
    /* owner, group 2000 (not a member): set-group-ID dropped, the rest set */
    expect("c_foreign_group_sgid", chmod_(D "/mine2000", 06755), 0);
    expect("c_foreign_group_sgid_dropped", mode_of(D "/mine2000"), S_IFREG | 04755);
    /* also on a directory */
    expect("c_dir_sgid", chmod_(D "/dir2000", 03775), 0);
    expect("c_dir_sgid_dropped", mode_of(D "/dir2000"), S_IFDIR | 01775);
    /* owner, group 3000 (a supplementary group): kept */
    expect("c_supp_group_sgid", chmod_(D "/mine3000", 02750), 0);
    expect("c_supp_group_sgid_kept", mode_of(D "/mine3000"), S_IFREG | 02750);
    /* a missing name in a directory the user may search: ENOENT first */
    expect("c_missing", chmod_(D "/nope", 0600), -ENOENT);
    /* a directory the user may not search: EACCES before EPERM */
    expect("c_closed_eacces", chmod_(D "/closed/theirs", 0600), -EACCES);
    /* read-only mount: EROFS before the ownership check */
    expect("c_rofs_theirs", chmod_(D "/m/theirs", 0600), -EROFS);
    expect("c_rofs_mine", chmod_(D "/m/mine", 0600), -EROFS);
    return finish();
}

int test_main(void) {
    a_block_sigchld();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_theirs", a_make_file(D "/theirs", "x", 0666), 0);
    expect("setup_theirs_own", chown_(D "/theirs", 2000, 2000), 0);
    expect("setup_theirs_mode", chmod_(D "/theirs", 0666), 0);
    expect("setup_grpfile", a_make_file(D "/grpfile", "x", 0664), 0);
    expect("setup_grpfile_own", chown_(D "/grpfile", 2000, 1000), 0);
    expect("setup_mine", a_make_file(D "/mine", "x", 0644), 0);
    expect("setup_mine_own", chown_(D "/mine", 1000, 1000), 0);
    expect("setup_mine2000", a_make_file(D "/mine2000", "x", 0644), 0);
    expect("setup_mine2000_own", chown_(D "/mine2000", 1000, 2000), 0);
    expect("setup_mine3000", a_make_file(D "/mine3000", "x", 0644), 0);
    expect("setup_mine3000_own", chown_(D "/mine3000", 1000, 3000), 0);
    expect("setup_dir2000", a_mkdir(D "/dir2000", 0755), 0);
    expect("setup_dir2000_own", chown_(D "/dir2000", 1000, 2000), 0);
    expect("setup_closed", a_mkdir(D "/closed", 0700), 0);
    expect("setup_closed_f", a_make_file(D "/closed/theirs", "x", 0644), 0);
    expect("setup_m", a_mkdir(D "/m", 0755), 0);
    expect("mount", syscall5(SYS_mount, "none", D "/m", "tmpfs", 0, "mode=0755"), 0);
    expect("setup_m_theirs", a_make_file(D "/m/theirs", "x", 0644), 0);
    expect("setup_m_theirs_own", chown_(D "/m/theirs", 2000, 2000), 0);
    expect("setup_m_mine", a_make_file(D "/m/mine", "x", 0644), 0);
    expect("setup_m_mine_own", chown_(D "/m/mine", 1000, 1000), 0);
    expect("remount_ro", syscall5(SYS_mount, "none", D "/m", "tmpfs", MS_REMOUNT | MS_RDONLY, 0), 0);
    expect("root_rofs", chmod_(D "/m/mine", 0600), -EROFS);

    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(child_user());
    i32 status = -1;
    expect("wait", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_passed", status, 0);

    /* Root (CAP_FOWNER, CAP_FSETID) changes anyone's mode and keeps
     * set-group-ID whatever the group. */
    expect("root_theirs", chmod_(D "/theirs", 02711), 0);
    expect("root_theirs_mode", mode_of(D "/theirs"), S_IFREG | 02711);

    expect("umount", syscall2(SYS_umount2, D "/m", 0), 0);
    expect("cleanup_m", a_rmdir(D "/m"), 0);
    expect("cleanup_closed_f", a_unlink(D "/closed/theirs"), 0);
    expect("cleanup_closed", a_rmdir(D "/closed"), 0);
    expect("cleanup_dir2000", a_rmdir(D "/dir2000"), 0);
    expect("cleanup_mine3000", a_unlink(D "/mine3000"), 0);
    expect("cleanup_mine2000", a_unlink(D "/mine2000"), 0);
    expect("cleanup_mine", a_unlink(D "/mine"), 0);
    expect("cleanup_grpfile", a_unlink(D "/grpfile"), 0);
    expect("cleanup_theirs", a_unlink(D "/theirs"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
