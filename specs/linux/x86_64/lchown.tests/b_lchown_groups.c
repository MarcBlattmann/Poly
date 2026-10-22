/* lchown(2) by an unprivileged owner with supplementary groups: the gid
 * may be set to the caller's file system gid or any supplementary group,
 * nothing else; the uid only to the caller's own. The checks use the file
 * system uid (which follows the effective uid unless setfsuid changed
 * it). A non-owner may pass -1 for both ids on a plain file, but not on
 * one whose set-ID bits the call would have to clear (EPERM). Root sets
 * up the files, then the checks run in a child with uid 4100 (saved uid
 * 4101), gid 4200 and supplementary groups 4300 and 4400.
 * poly-diff: needs=root (setgroups and set*id to other users)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_lchown 94
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_setfsuid 122
#define D "/tmp/b_lchown_groups"

static i64 lchown(const char *p, i64 uid, i64 gid) { return syscall3(SYS_lchown, p, uid, gid); }

static int ids(const char *p, u32 uid, u32 gid) {
    struct a_stat st;
    return a_lstat_path(p, &st) == 0 && st.st_uid == uid && st.st_gid == gid;
}
static i64 mode_of(const char *p) {
    struct a_stat st;
    if (a_lstat_path(p, &st) != 0) return -1;
    return st.st_mode & 07777;
}

static int child(void) {
    u32 groups[2] = {4300, 4400};
    expect("setgroups", syscall2(SYS_setgroups, 2, groups), 0);
    expect("setresgid", syscall3(SYS_setresgid, 4200, 4200, 4200), 0);
    expect("setresuid", syscall3(SYS_setresuid, 4100, 4100, 4101), 0);

    expect("own_gid_egid", lchown(D "/mine", -1, 4200), 0);
    check("own_gid_egid_ids", ids(D "/mine", 4100, 4200), 0);
    expect("own_gid_supp1", lchown(D "/mine", -1, 4300), 0);
    expect("own_gid_supp2", lchown(D "/mine", 4100, 4400), 0);
    check("own_gid_supp_ids", ids(D "/mine", 4100, 4400), 0);
    expect("own_gid_other_eperm", lchown(D "/mine", -1, 4500), -EPERM);
    expect("own_uid_other_eperm", lchown(D "/mine", 4101, -1), -EPERM);
    expect("own_uid_saved_eperm", lchown(D "/mine", 4101, 4300), -EPERM);
    check("eperm_ids_kept", ids(D "/mine", 4100, 4400), 0);
    /* A file of the caller's in a group the caller is not in. */
    expect("foreign_group_to_supp", lchown(D "/mine_g", -1, 4300), 0);
    check("foreign_group_to_supp_ids", ids(D "/mine_g", 4100, 4300), 0);
    /* The caller's own link to a file of somebody else. */
    expect("own_link", lchown(D "/mylink", 4100, 4400), 0);
    check("own_link_ids", ids(D "/mylink", 4100, 4400), 0);
    check("own_link_target_kept", ids(D "/theirs", 4101, 4500), 0);

    /* Files of uid 4101 (not the caller). */
    expect("theirs_supp_gid_eperm", lchown(D "/theirs", -1, 4300), -EPERM);
    expect("theirs_own_uid_eperm", lchown(D "/theirs", 4100, -1), -EPERM);
    expect("theirs_same_ids_eperm", lchown(D "/theirs", 4101, 4500), -EPERM);
    expect("theirs_minus1_ok", lchown(D "/theirs", -1, -1), 0);
    expect("theirs_suid_minus1_eperm", lchown(D "/theirs_suid", -1, -1), -EPERM);
    expect("theirs_suid_mode_kept", mode_of(D "/theirs_suid"), 04755);
    expect("theirs_sgid_gx_minus1_eperm", lchown(D "/theirs_sgid", -1, -1), -EPERM);
    expect("theirs_sgid_no_gx_minus1_ok", lchown(D "/theirs_sgid_nogx", -1, -1), 0);
    expect("theirs_sgid_no_gx_kept", mode_of(D "/theirs_sgid_nogx"), 02745);

    /* With the file system uid 4101 (allowed: it is the saved uid), the
     * files of 4101 are the caller's and its own are not. */
    syscall1(SYS_setfsuid, 4101);
    expect("fsuid_is_4101", syscall1(SYS_setfsuid, -1), 4101);
    expect("fsuid_theirs_gid", lchown(D "/theirs", -1, 4300), 0);
    check("fsuid_theirs_gid_ids", ids(D "/theirs", 4101, 4300), 0);
    expect("fsuid_theirs_suid_minus1", lchown(D "/theirs_suid", -1, -1), 0);
    expect("fsuid_theirs_suid_cleared", mode_of(D "/theirs_suid"), 0755);
    expect("fsuid_mine_eperm", lchown(D "/mine", -1, 4300), -EPERM);
    expect("fsuid_mine_to_fsuid_eperm", lchown(D "/mine", 4101, -1), -EPERM);
    syscall1(SYS_setfsuid, 4100);
    expect("fsuid_back", lchown(D "/mine", -1, 4300), 0);
    return finish();
}

static void cleanup(void) {
    const char *names[] = {"/mylink", "/theirs_sgid_nogx", "/theirs_sgid", "/theirs_suid", "/theirs", "/mine_g", "/mine"};
    static char path[128];
    for (unsigned i = 0; i < sizeof names / sizeof names[0]; i++) a_unlink(str_cat2(path, D, names[i]));
    a_rmdir(D);
}

int test_main(void) {
    int ok = a_mkdir(D, 0777) == 0 && syscall2(SYS_chmod, D, 0777) == 0;
    ok &= a_make_file(D "/mine", "", 0644) == 0 && lchown(D "/mine", 4100, 4100) == 0;
    ok &= a_make_file(D "/mine_g", "", 0644) == 0 && lchown(D "/mine_g", 4100, 4500) == 0;
    ok &= a_make_file(D "/theirs", "", 0644) == 0 && lchown(D "/theirs", 4101, 4500) == 0;
    ok &= a_make_file(D "/theirs_suid", "", 0644) == 0 && lchown(D "/theirs_suid", 4101, 4500) == 0 &&
          syscall2(SYS_chmod, D "/theirs_suid", 04755) == 0;
    ok &= a_make_file(D "/theirs_sgid", "", 0644) == 0 && lchown(D "/theirs_sgid", 4101, 4500) == 0 &&
          syscall2(SYS_chmod, D "/theirs_sgid", 02755) == 0;
    ok &= a_make_file(D "/theirs_sgid_nogx", "", 0644) == 0 && lchown(D "/theirs_sgid_nogx", 4101, 4500) == 0 &&
          syscall2(SYS_chmod, D "/theirs_sgid_nogx", 02745) == 0;
    ok &= syscall3(SYS_symlinkat, "theirs", AT_FDCWD, D "/mylink") == 0 && lchown(D "/mylink", 4100, 4100) == 0;
    check("setup", ok, 0);
    if (!ok) {
        cleanup();
        return finish();
    }

    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, child());
    i32 st = -1;
    if (pid < 0 || syscall4(SYS_wait4, pid, &st, 0, 0) != pid) st = -1;
    cleanup();
    return st == 0 ? 0 : 1;
}
