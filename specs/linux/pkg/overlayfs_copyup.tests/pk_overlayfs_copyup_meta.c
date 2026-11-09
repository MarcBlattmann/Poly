/* Changing only the metadata of a lower-layer entry through the merged view
 * (chmod, chown, utimensat, setxattr, removexattr) copies the entry to the
 * upper layer first (with metacopy=off, as set here, the data too). The copy
 * keeps the lower entry's mode, owner, times and content apart from the
 * change; the lower entry is untouched. A directory is copied without its
 * children, which still show through. Extended attributes are visible through
 * the merged view except the overlay's own (trusted.overlay.*), which are
 * hidden from get and list. chown clears set-user-ID on a file.
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN; trusted.* attributes)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_ovc_meta"
#define M B "/m"
#define U B "/u"
#define L B "/l"
#define T1 1000000000L

static i64 times(const char *path, i64 asec, i64 msec) {
    i64 ts[4] = {asec, 0, msec, 0};
    return syscall4(SYS_utimensat, AT_FDCWD, path, ts, 0);
}

static int body(void) {
    expect("tmpfs", pm_tmpfs(B), 0);
    pm_mkdirs(L "/dir1/sub");
    pm_mkdirs(L "/sg");
    pm_mkdirs(U);
    pm_mkdirs(B "/w");
    pm_mkdirs(M);
    const char *names[] = {"f1", "f2", "f3", "f4", "f5", "f6", "suid"};
    for (int i = 0; i < 7; i++) {
        char p[64];
        pm_path(p, L "/", names[i]);
        a_make_file(p, "data", 0644);
        syscall3(SYS_chown, p, 100, 200);
        times(p, T1, T1);
    }
    syscall3(SYS_chmod, L "/suid", 04755, 0);
    pm_put(L, "/dir1/child", "c", 0644);
    syscall3(SYS_chown, L "/dir1", 100, 200);
    syscall3(SYS_chmod, L "/dir1", 0750, 0);
    times(L "/dir1", T1, T1);
    syscall3(SYS_chmod, L "/sg", 02755, 0);
    syscall4(SYS_lsetxattr, L "/f6", "trusted.pk_low", "lowval", 6);

    const char *opts = "lowerdir=" L ",upperdir=" U ",workdir=" B "/w,metacopy=off,redirect_dir=off,index=off";
    expect("mount", pm_mount("ov", M, "overlay", 0, opts), 0);
    struct a_stat st;

    /* chmod */
    expect("chmod", syscall3(SYS_chmod, M "/f1", 0600, 0), 0);
    expect("chmod_copied", a_stat_path(U "/f1", &st), 0);
    expect("chmod_upper_mode", st.st_mode, S_IFREG | 0600);
    expect("chmod_upper_uid", st.st_uid, 100);
    expect("chmod_upper_gid", st.st_gid, 200);
    expect("chmod_upper_mtime", st.st_mtime_sec, T1);
    expect("chmod_upper_size", st.st_size, 4);
    pm_expect_file("chmod_upper_data", U "/f1", "data");
    expect("chmod_lower_mode", pm_mode(L "/f1"), S_IFREG | 0644);
    expect("chmod_merged_mode", pm_mode(M "/f1"), S_IFREG | 0600);
    expect("chmod_merged_stat", a_stat_path(M "/f1", &st), 0);
    expect("chmod_merged_mtime", st.st_mtime_sec, T1);

    /* chown: one id at a time (-1 keeps) */
    expect("chown_uid_only", syscall3(SYS_chown, M "/f2", 300, -1), 0);
    expect("chown_uid_stat", a_stat_path(U "/f2", &st), 0);
    expect("chown_uid_new", st.st_uid, 300);
    expect("chown_uid_gid_kept", st.st_gid, 200);
    expect("chown_gid_only", syscall3(SYS_chown, M "/f3", -1, 400), 0);
    expect("chown_gid_stat", a_stat_path(U "/f3", &st), 0);
    expect("chown_gid_uid_kept", st.st_uid, 100);
    expect("chown_gid_new", st.st_gid, 400);
    expect("chown_lower_untouched", a_stat_path(L "/f2", &st), 0);
    expect("chown_lower_uid", st.st_uid, 100);
    expect("chown_suid", syscall3(SYS_chown, M "/suid", 5, 6), 0);
    expect("chown_suid_stat", a_stat_path(M "/suid", &st), 0);
    expect("chown_clears_suid", st.st_mode & S_ISUID, 0);
    expect("chown_suid_lower_keeps", pm_mode(L "/suid") & S_ISUID, S_ISUID);

    /* timestamps */
    expect("utimensat", times(M "/f4", 5000000, 6000000), 0);
    expect("utimens_upper", a_stat_path(U "/f4", &st), 0);
    expect("utimens_upper_atime", st.st_atime_sec, 5000000);
    expect("utimens_upper_mtime", st.st_mtime_sec, 6000000);
    expect("utimens_merged", a_stat_path(M "/f4", &st), 0);
    expect("utimens_merged_mtime", st.st_mtime_sec, 6000000);
    expect("utimens_lower", a_stat_path(L "/f4", &st), 0);
    expect("utimens_lower_mtime", st.st_mtime_sec, T1);

    /* extended attributes */
    char v[16];
    memset(v, 0, sizeof v);
    expect("setxattr", syscall5(SYS_lsetxattr, M "/f5", "trusted.pk", "v1", 2, 0), 0);
    expect("setxattr_copied", pm_exists(U "/f5"), 1);
    expect("getxattr_merged", syscall4(SYS_lgetxattr, M "/f5", "trusted.pk", v, sizeof v), 2);
    check("getxattr_value", v[0] == 'v' && v[1] == '1', v[0]);
    expect("getxattr_lower_none", syscall4(SYS_lgetxattr, L "/f5", "trusted.pk", v, sizeof v), -ENODATA);
    expect("getxattr_overlay_hidden", syscall4(SYS_lgetxattr, M "/f5", "trusted.overlay.origin", v, sizeof v), -ENODATA);
    char list[256];
    i64 n = syscall3(SYS_llistxattr, M "/f5", list, sizeof list);
    check("listxattr", n > 0, n);
    int hidden = 1, found = 0;
    for (i64 i = 0; i < n;) {
        if (pm_find(list + i, "trusted.overlay") == 0) hidden = 0;
        if (str_eq(list + i, "trusted.pk")) found = 1;
        i += (i64)str_len(list + i) + 1;
    }
    check("listxattr_has_ours", found, 0);
    check("listxattr_hides_overlay", hidden, 0);
    expect("lower_xattr_visible", syscall4(SYS_lgetxattr, M "/f6", "trusted.pk_low", v, sizeof v), 6);
    expect("removexattr", syscall2(SYS_lremovexattr, M "/f6", "trusted.pk_low"), 0);
    expect("removed_merged", syscall4(SYS_lgetxattr, M "/f6", "trusted.pk_low", v, sizeof v), -ENODATA);
    expect("removed_lower_keeps", syscall4(SYS_lgetxattr, L "/f6", "trusted.pk_low", v, sizeof v), 6);

    /* directories: copied without children, metadata kept */
    expect("chmod_dir", syscall3(SYS_chmod, M "/dir1", 0700, 0), 0);
    expect("dir_copied", a_stat_path(U "/dir1", &st), 0);
    expect("dir_upper_mode", st.st_mode, S_IFDIR | 0700);
    expect("dir_upper_uid", st.st_uid, 100);
    expect("dir_upper_gid", st.st_gid, 200);
    expect("dir_upper_mtime", st.st_mtime_sec, T1);
    pm_expect_list("dir_upper_empty", U "/dir1", "");
    pm_expect_list("dir_merged_children", M "/dir1", "child,sub");
    pm_expect_file("dir_child_visible", M "/dir1/child", "c");
    expect("dir_lower_mode", pm_mode(L "/dir1"), S_IFDIR | 0750);
    expect("chown_dir_keeps_setgid", syscall3(SYS_chown, M "/sg", 7, 8), 0);
    expect("setgid_dir_stat", a_stat_path(U "/sg", &st), 0);
    expect("setgid_dir_mode", st.st_mode, S_IFDIR | 02755);
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
