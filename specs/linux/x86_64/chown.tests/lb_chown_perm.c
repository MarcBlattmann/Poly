/* chown(2) rules for a user other than root: the owner may set the group to
 * its effective gid or a supplementary group, nothing else; a non-owner
 * gets EPERM for any real change, but chown(-1, -1) succeeds on a file it
 * does not own unless that would clear a set-user-ID/set-group-ID bit.
 * Read-only mounts answer EROFS first.
 * poly-diff: needs=root (chowns files to other users, changes uid, mounts)
 */
#include "a_sys.h"

#define D "/tmp/lb_chown_perm"
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_mount 165
#define SYS_umount2 166
#define MS_RDONLY 1
#define MS_REMOUNT 32

static i64 chown_(const char *p, i64 u, i64 g) { return syscall3(SYS_chown, p, u, g); }
static void ids(const char *p, i64 *u, i64 *g) {
    struct a_stat st;
    *u = *g = -1;
    if (a_stat_path(p, &st) == 0) *u = st.st_uid, *g = st.st_gid;
}
static u32 perm_of(const char *p) {
    struct a_stat st;
    if (a_stat_path(p, &st)) return 0xffffffff;
    return st.st_mode & 07777;
}

static int child_user(void) {
    i64 u, g;
    u32 groups[2] = {3000, 3001};
    expect("c_setgroups", syscall2(SYS_setgroups, 2, groups), 0);
    expect("c_setresgid", syscall3(SYS_setresgid, 1000, 1000, 1000), 0);
    expect("c_setresuid", syscall3(SYS_setresuid, 1000, 1000, 1000), 0);
    /* the owner: group to its gid or a supplementary group */
    expect("c_group_to_supp", chown_(D "/mine", -1, 3000), 0);
    ids(D "/mine", &u, &g);
    expect("c_group_now_3000", g, 3000);
    expect("c_group_to_supp2", chown_(D "/mine", 1000, 3001), 0);
    expect("c_group_to_egid", chown_(D "/mine", -1, 1000), 0);
    expect("c_group_foreign_eperm", chown_(D "/mine", -1, 2000), -EPERM);
    expect("c_uid_other_eperm", chown_(D "/mine", 2000, -1), -EPERM);
    expect("c_uid_root_eperm", chown_(D "/mine", 0, 1000), -EPERM);
    ids(D "/mine", &u, &g);
    expect("c_mine_uid_kept", u, 1000);
    expect("c_mine_gid_kept", g, 1000);
    /* the owner may keep (and name again) a group it is not in */
    expect("c_keep_foreign_group_noop", chown_(D "/mine2000", 1000, -1), 0);
    expect("c_set_same_foreign_group_ok", chown_(D "/mine2000", -1, 2000), 0);
    expect("c_other_foreign_group_eperm", chown_(D "/mine2000", -1, 2001), -EPERM);
    /* not the owner */
    expect("c_theirs_take", chown_(D "/theirs", 1000, -1), -EPERM);
    expect("c_theirs_same_ids", chown_(D "/theirs", 2000, 2000), -EPERM);
    expect("c_theirs_group_to_mine", chown_(D "/theirs", -1, 1000), -EPERM);
    expect("c_theirs_noop_ok", chown_(D "/theirs", -1, -1), 0);
    expect("c_group_member_file", chown_(D "/grp3000", -1, 1000), -EPERM);
    /* -1/-1 on a set-user-ID file of someone else: clearing the bit is a
     * mode change, refused */
    expect("c_theirs_suid_noop_eperm", chown_(D "/theirs_suid", -1, -1), -EPERM);
    expect("c_theirs_suid_kept", perm_of(D "/theirs_suid"), 04755);
    /* EACCES on the way comes first, EROFS before EPERM */
    expect("c_closed_eacces", chown_(D "/closed/f", -1, -1), -EACCES);
    expect("c_rofs_theirs", chown_(D "/m/theirs", 1000, -1), -EROFS);
    expect("c_rofs_noop", chown_(D "/m/theirs", -1, -1), -EROFS);
    return finish();
}

int test_main(void) {
    i64 u, g;
    a_block_sigchld();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_mine", a_make_file(D "/mine", "x", 0644), 0);
    expect("setup_mine_own", chown_(D "/mine", 1000, 1000), 0);
    expect("setup_mine2000", a_make_file(D "/mine2000", "x", 0644), 0);
    expect("setup_mine2000_own", chown_(D "/mine2000", 1000, 2000), 0);
    expect("setup_theirs", a_make_file(D "/theirs", "x", 0666), 0);
    expect("setup_theirs_own", chown_(D "/theirs", 2000, 2000), 0);
    expect("setup_grp3000", a_make_file(D "/grp3000", "x", 0664), 0);
    expect("setup_grp3000_own", chown_(D "/grp3000", 2000, 3000), 0);
    expect("setup_theirs_suid", a_make_file(D "/theirs_suid", "x", 0644), 0);
    expect("setup_theirs_suid_own", chown_(D "/theirs_suid", 2000, 2000), 0);
    expect("setup_theirs_suid_mode", syscall2(SYS_chmod, D "/theirs_suid", 04755), 0);
    expect("setup_closed", a_mkdir(D "/closed", 0700), 0);
    expect("setup_closed_f", a_make_file(D "/closed/f", "x", 0644), 0);
    expect("setup_m", a_mkdir(D "/m", 0755), 0);
    expect("mount", syscall5(SYS_mount, "none", D "/m", "tmpfs", 0, "mode=0755"), 0);
    expect("setup_m_theirs", a_make_file(D "/m/theirs", "x", 0644), 0);
    expect("setup_m_theirs_own", chown_(D "/m/theirs", 2000, 2000), 0);
    expect("remount_ro", syscall5(SYS_mount, "none", D "/m", "tmpfs", MS_REMOUNT | MS_RDONLY, 0), 0);
    expect("root_rofs", chown_(D "/m/theirs", 0, 0), -EROFS);

    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(child_user());
    i32 status = -1;
    expect("wait", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_passed", status, 0);

    /* root (CAP_CHOWN) sets anything, and still loses set-user-ID */
    expect("root_give", chown_(D "/theirs_suid", 5000, 6000), 0);
    ids(D "/theirs_suid", &u, &g);
    expect("root_give_uid", u, 5000);
    expect("root_give_gid", g, 6000);
    expect("root_give_suid_cleared", perm_of(D "/theirs_suid"), 0755);

    expect("umount", syscall2(SYS_umount2, D "/m", 0), 0);
    expect("cleanup_m", a_rmdir(D "/m"), 0);
    expect("cleanup_closed_f", a_unlink(D "/closed/f"), 0);
    expect("cleanup_closed", a_rmdir(D "/closed"), 0);
    expect("cleanup_theirs_suid", a_unlink(D "/theirs_suid"), 0);
    expect("cleanup_grp3000", a_unlink(D "/grp3000"), 0);
    expect("cleanup_theirs", a_unlink(D "/theirs"), 0);
    expect("cleanup_mine2000", a_unlink(D "/mine2000"), 0);
    expect("cleanup_mine", a_unlink(D "/mine"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
