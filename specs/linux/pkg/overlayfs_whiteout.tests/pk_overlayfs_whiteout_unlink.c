/* Removing a name that a lower layer has leaves a whiteout in the upper
 * layer: a character device with number 0:0 and mode 0 (lstat: S_IFCHR,
 * st_rdev 0), invisible through the merged view (ENOENT, absent from
 * readdir). Removing a name that only the upper layer has leaves nothing.
 * A name that was copied up and then removed gets a whiteout too (the lower
 * layer still has it). Creating the name again replaces the whiteout with the
 * new object; the lower file stays hidden. A whiteout is also left for names
 * in a lower layer below another lower layer. unlink(2) of a whiteout through
 * the merged view is ENOENT.
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_ovw_unlink"
#define M B "/m"
#define U B "/u"
#define L1 B "/l1"
#define L2 B "/l2"

static void check_whiteout(const char *name, const char *path) {
    struct a_stat st;
    i64 r = a_lstat_path(path, &st);
    check(name, r == 0 && (st.st_mode & S_IFMT) == S_IFCHR && st.st_rdev == 0 && (st.st_mode & 07777) == 0, r);
}

static int body(void) {
    expect("tmpfs", pm_tmpfs(B), 0);
    pm_mkdirs(L1);
    pm_mkdirs(L2);
    pm_mkdirs(U);
    pm_mkdirs(B "/w");
    pm_mkdirs(M);
    pm_put(L1, "/in_l1", "l1", 0644);
    pm_put(L2, "/in_l2", "l2", 0644);
    pm_put(L2, "/both", "l2-both", 0644);
    pm_put(L1, "/both", "l1-both", 0644);
    pm_put(L1, "/copied", "orig", 0644);
    pm_put(L1, "/replaced", "orig-replaced", 0644);
    pm_put(U, "/up_only", "up", 0644);
    pm_put(L1, "/sym_target", "t", 0644);
    syscall3(SYS_symlink, "sym_target", L1 "/sym", 0);
    pm_mknod(L1 "/fifo", S_IFIFO | 0644, 0, 0);

    const char *opts = "lowerdir=" L1 ":" L2 ",upperdir=" U ",workdir=" B "/w,index=off,metacopy=off";
    expect("mount", pm_mount("ov", M, "overlay", 0, opts), 0);

    expect("unlink_l1", a_unlink(M "/in_l1"), 0);
    check_whiteout("l1_whiteout", U "/in_l1");
    expect("l1_hidden", pm_exists(M "/in_l1"), 0);
    expect("l1_open_enoent", a_open(M "/in_l1", O_RDONLY, 0), -ENOENT);
    pm_expect_file("l1_lower_intact", L1 "/in_l1", "l1");
    expect("unlink_l2", a_unlink(M "/in_l2"), 0);
    check_whiteout("l2_whiteout", U "/in_l2");
    expect("l2_hidden", pm_exists(M "/in_l2"), 0);
    pm_expect_file("l2_lower_intact", L2 "/in_l2", "l2");

    /* the name exists in two lowers: one whiteout hides both */
    expect("unlink_both", a_unlink(M "/both"), 0);
    check_whiteout("both_whiteout", U "/both");
    expect("both_hidden", pm_exists(M "/both"), 0);

    /* link and fifo */
    expect("unlink_symlink", a_unlink(M "/sym"), 0);
    check_whiteout("symlink_whiteout", U "/sym");
    pm_expect_file("symlink_target_remains", M "/sym_target", "t");
    expect("unlink_fifo", a_unlink(M "/fifo"), 0);
    check_whiteout("fifo_whiteout", U "/fifo");

    /* upper only: nothing left behind */
    expect("unlink_upper_only", a_unlink(M "/up_only"), 0);
    expect("upper_only_no_trace", pm_exists(U "/up_only"), 0);

    /* copied up, then removed */
    expect("copy_up", a_make_file(M "/copied", "changed", 0644), 0);
    pm_expect_file("copy_up_in_upper", U "/copied", "changed");
    expect("unlink_copied", a_unlink(M "/copied"), 0);
    check_whiteout("copied_whiteout", U "/copied");
    pm_expect_file("copied_lower_intact", L1 "/copied", "orig");

    /* the listing shows none of them */
    pm_expect_list("readdir_merged", M, "replaced,sym_target");
    pm_expect_list("upper_listing", U, "both,copied,fifo,in_l1,in_l2,sym");

    /* re-creating a name replaces the whiteout */
    expect("recreate_removed", a_make_file(M "/in_l1", "brand-new", 0600), 0);
    pm_expect_file("recreated_content", M "/in_l1", "brand-new");
    expect("recreated_is_regular", pm_mode(U "/in_l1"), S_IFREG | 0600);
    pm_expect_file("lower_still_old", L1 "/in_l1", "l1");
    expect("unlink_replaced", a_unlink(M "/replaced"), 0);
    expect("recreate_as_symlink", syscall3(SYS_symlink, "elsewhere", M "/replaced", 0), 0);
    expect("recreated_symlink", pm_mode(U "/replaced") & S_IFMT, S_IFLNK);
    expect("recreate_both", a_mkdir(M "/both", 0711), 0);
    pm_expect_list("recreate_both_empty", M "/both", "");
    expect("recreate_both_mode", pm_mode(M "/both"), S_IFDIR | 0711);

    /* removing again */
    expect("unlink_twice", a_unlink(M "/in_l2"), -ENOENT);
    expect("unlink_missing", a_unlink(M "/never"), -ENOENT);
    expect("umount", pm_umount(M, 0), 0);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_rmdir(B);
    pm_in_ns(body);
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
