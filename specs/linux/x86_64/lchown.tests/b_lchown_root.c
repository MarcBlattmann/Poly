/* lchown(2) as root (CAP_CHOWN, CAP_FOWNER): any uid and gid up to
 * 2^32-2 can be set, on files root does not own as well; -1 leaves an id
 * as it is. A symbolic link gets the ids, its target keeps its own. The
 * set-ID bits of a regular file are cleared as for any other caller
 * (set-group-ID only with group execute), also on a file of another user
 * and also with -1 for both ids; a directory keeps them.
 * poly-diff: needs=root (only root may give a file away)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_lchown 94
#define D "/tmp/b_lchown_root"
#define F D "/f"

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

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(F, "", 0644), 0);

    expect("give_away", lchown(F, 4242, 4343), 0);
    check("give_away_ids", ids(F, 4242, 4343), 0);
    expect("uid_only", lchown(F, 11, -1), 0);
    check("uid_only_ids", ids(F, 11, 4343), 0);
    expect("gid_only", lchown(F, -1, 12), 0);
    check("gid_only_ids", ids(F, 11, 12), 0);
    expect("largest_ids", lchown(F, 4294967294L, 4294967294L), 0);
    check("largest_ids_ids", ids(F, 4294967294U, 4294967294U), 0);
    expect("high_bits_ignored", lchown(F, (1L << 32) | 21, (3L << 32) | 22), 0);
    check("high_bits_ids", ids(F, 21, 22), 0);
    expect("back_to_root", lchown(F, 0, 0), 0);
    check("back_to_root_ids", ids(F, 0, 0), 0);

    /* Set-ID bits. */
    expect("setup_suid", syscall2(SYS_chmod, F, 06755), 0);
    expect("suid_uid_change", lchown(F, 31, -1), 0);
    expect("suid_uid_change_cleared", mode_of(F), 0755);
    expect("setup_suid_other", syscall2(SYS_chmod, F, 06755), 0);
    expect("suid_other_owner_minus1", lchown(F, -1, -1), 0);
    expect("suid_other_owner_minus1_cleared", mode_of(F), 0755);
    expect("setup_sgid_no_gx", syscall2(SYS_chmod, F, 06745), 0);
    expect("sgid_no_gx_change", lchown(F, -1, 32), 0);
    expect("sgid_no_gx_kept", mode_of(F), 02745);
    expect("setup_sticky", syscall2(SYS_chmod, F, 01755), 0);
    expect("sticky_change", lchown(F, 0, 0), 0);
    expect("sticky_kept", mode_of(F), 01755);
    expect("setup_dir", a_mkdir(D "/dir", 0755), 0);
    expect("setup_dir_bits", syscall2(SYS_chmod, D "/dir", 07755), 0);
    expect("dir_change", lchown(D "/dir", 41, 42), 0);
    expect("dir_bits_kept", mode_of(D "/dir"), 07755);
    check("dir_ids", ids(D "/dir", 41, 42), 0);

    /* Links. */
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    expect("setup_dangling", syscall3(SYS_symlinkat, "gone", AT_FDCWD, D "/dl"), 0);
    expect("setup_dirlink", syscall3(SYS_symlinkat, "dir", AT_FDCWD, D "/sl"), 0);
    expect("link", lchown(D "/l", 51, 52), 0);
    check("link_ids", ids(D "/l", 51, 52), 0);
    check("link_target_ids_kept", ids(F, 0, 0), 0);
    expect("link_uid_only", lchown(D "/l", 53, -1), 0);
    check("link_uid_only_ids", ids(D "/l", 53, 52), 0);
    expect("dangling", lchown(D "/dl", 54, 55), 0);
    check("dangling_ids", ids(D "/dl", 54, 55), 0);
    expect("dirlink_slash", lchown(D "/sl/", 61, 62), 0);
    check("dirlink_slash_dir_ids", ids(D "/dir", 61, 62), 0);
    check("dirlink_slash_link_ids_kept", ids(D "/sl", 0, 0), 0);
    expect("dirlink_itself", lchown(D "/sl", 63, 64), 0);
    check("dirlink_itself_ids", ids(D "/sl", 63, 64), 0);
    check("dirlink_itself_dir_kept", ids(D "/dir", 61, 62), 0);

    expect("cleanup_sl", a_unlink(D "/sl"), 0);
    expect("cleanup_dl", a_unlink(D "/dl"), 0);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_dir", a_rmdir(D "/dir"), 0);
    expect("cleanup_f", a_unlink(F), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
