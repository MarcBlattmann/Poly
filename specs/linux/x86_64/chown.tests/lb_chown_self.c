/* chown(2) by the owner: -1 (0xffffffff) leaves an id alone; the owner may
 * "change" to its own uid and to its own gid; a normal user may not give
 * the file away (EPERM) nor pick a group it is not in; root may set any
 * ids. Symbolic links are followed. ctime changes, also for chown(-1, -1). */
#include "a_sys.h"

#define D "/tmp/lb_chown_self"

static i64 chown_(const char *p, i64 u, i64 g) { return syscall3(SYS_chown, p, u, g); }

int test_main(void) {
    i64 uid = syscall0(SYS_getuid), gid = syscall0(SYS_getgid);
    int root = uid == 0;
    struct a_stat st, st2;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_link", syscall2(SYS_symlink, "f", D "/l"), 0);
    expect("stat_new", a_stat_path(D "/f", &st), 0);
    expect("new_file_uid_is_caller", st.st_uid, uid);
    expect("new_file_gid_is_caller", st.st_gid, gid);

    expect("both_minus1", chown_(D "/f", -1, -1), 0);
    expect("both_0xffffffff", chown_(D "/f", 0xffffffffL, 0xffffffffL), 0);
    expect("own_ids", chown_(D "/f", uid, gid), 0);
    expect("own_uid_only", chown_(D "/f", uid, -1), 0);
    expect("own_gid_only", chown_(D "/f", -1, gid), 0);
    expect("stat_same", a_stat_path(D "/f", &st), 0);
    expect("uid_same", st.st_uid, uid);
    expect("gid_same", st.st_gid, gid);

    /* Giving it away, or a foreign group. */
    expect("give_away", chown_(D "/f", 4321, -1), root ? 0 : -EPERM);
    expect("foreign_group", chown_(D "/f", -1, 4322), root ? 0 : -EPERM);
    expect("stat_after", a_stat_path(D "/f", &st), 0);
    expect("uid_after", st.st_uid, root ? 4321 : uid);
    expect("gid_after", st.st_gid, root ? 4322 : gid);
    /* the uid and gid are 32-bit: the upper half of the register is ignored */
    expect("upper_half_ignored", chown_(D "/f", (i64)0x700000000L | uid, (i64)0x700000000L | gid), 0);
    expect("stat_upper", a_stat_path(D "/f", &st), 0);
    expect("uid_upper", st.st_uid, uid);
    expect("gid_upper", st.st_gid, gid);
    /* 0xfffffffe is an ordinary id (only root may set it) */
    expect("uid_fffffffe", chown_(D "/f", 0xfffffffeL, -1), root ? 0 : -EPERM);
    expect("stat_fffffffe", a_stat_path(D "/f", &st), 0);
    expect("uid_fffffffe_value", st.st_uid, root ? 0xfffffffeL : uid);
    expect("back_to_caller", chown_(D "/f", uid, gid), 0);

    /* Through a symbolic link: the target changes, the link does not. */
    expect("link_lstat", a_lstat_path(D "/l", &st), 0);
    expect("via_link", chown_(D "/l", root ? 77 : uid, root ? 78 : gid), 0);
    expect("target_stat", a_stat_path(D "/f", &st2), 0);
    expect("target_uid", st2.st_uid, root ? 77 : uid);
    expect("target_gid", st2.st_gid, root ? 78 : gid);
    expect("link_lstat_after", a_lstat_path(D "/l", &st2), 0);
    expect("link_uid_unchanged", st2.st_uid, uid);
    expect("link_gid_unchanged", st2.st_gid, gid);
    expect("back_via_link", chown_(D "/l", uid, gid), 0);

    /* Directories. */
    expect("mkdir_sub", a_mkdir(D "/sub", 0755), 0);
    expect("dir_own", chown_(D "/sub/", uid, gid), 0);
    expect("dir_give_away", chown_(D "/sub", 4321, 4322), root ? 0 : -EPERM);

    /* ctime changes, even for -1/-1; mtime does not. */
    struct a_timespec ts[2] = {{1100000000, 0}, {1100000000, 0}};
    expect("utimensat", syscall4(SYS_utimensat, AT_FDCWD, D "/f", ts, 0), 0);
    expect("stat_t0", a_stat_path(D "/f", &st), 0);
    struct a_timespec pause = {0, 20000000};
    syscall2(SYS_nanosleep, &pause, 0);
    expect("chown_noop", chown_(D "/f", -1, -1), 0);
    expect("stat_t1", a_stat_path(D "/f", &st2), 0);
    check("ctime_advanced_noop", st2.st_ctime_sec > st.st_ctime_sec ||
                                     (st2.st_ctime_sec == st.st_ctime_sec && st2.st_ctime_nsec > st.st_ctime_nsec), 0);
    expect("mtime_kept", st2.st_mtime_sec, 1100000000);
    expect("atime_kept", st2.st_atime_sec, 1100000000);

    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
