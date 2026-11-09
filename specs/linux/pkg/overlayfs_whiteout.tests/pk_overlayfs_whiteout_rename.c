/* renameat2(RENAME_WHITEOUT) on the filesystem an overlay's layers live on
 * (tmpfs): the source is moved to the target name and a whiteout (character
 * device 0:0, mode 0) is created under the source name in one step. It needs
 * no privilege: a normal user may do it and may make a 0:0 character device
 * with mknod, while any other device node needs CAP_MKNOD (EPERM). With RENAME_NOREPLACE
 * it is allowed (EEXIST if the target exists); with RENAME_EXCHANGE it is
 * EINVAL. It works for directories and replaces an existing target like a
 * plain rename. The whiteout node cannot be opened (ENXIO), is a plain
 * directory entry (unlink removes it, rename moves it).
 * poly-diff: needs=root (mknod of a device node, mount of tmpfs)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_ovw_ren"
#define SYS_setresuid_ 117
#define SYS_setresgid_ 119

static i64 ren2(const char *a, const char *b, u32 flags) {
    return syscall5(SYS_renameat2, AT_FDCWD, a, AT_FDCWD, b, flags);
}

static int is_whiteout(const char *path) {
    struct a_stat st;
    if (a_lstat_path(path, &st) < 0) return 0;
    return (st.st_mode & S_IFMT) == S_IFCHR && st.st_rdev == 0 && (st.st_mode & 07777) == 0;
}

static int as_user(void) {
    syscall3(SYS_setresgid_, 65534, 65534, 65534);
    syscall3(SYS_setresuid_, 65534, 65534, 65534);
    expect("user_rename_whiteout", ren2(B "/u/src", B "/u/dst", RENAME_WHITEOUT), 0);
    expect("user_src_is_whiteout", is_whiteout(B "/u/src"), 1);
    pm_expect_file("user_dst_content", B "/u/dst", "user-file");
    struct a_stat st;
    expect("user_whiteout_stat", a_lstat_path(B "/u/src", &st), 0);
    expect("user_whiteout_owner", st.st_uid, 65534);
    expect("user_mknod_whiteout", pm_mknod(B "/u/wh", S_IFCHR, 0, 0), 0);
    expect("user_mknod_whiteout_ok", is_whiteout(B "/u/wh"), 1);
    expect("user_mknod_other_chardev_eperm", pm_mknod(B "/u/dev", S_IFCHR | 0600, 1, 3), -EPERM);
    expect("user_mknod_blockdev_eperm", pm_mknod(B "/u/blk", S_IFBLK | 0600, 7, 0), -EPERM);
    expect("user_mknod_fifo_ok", pm_mknod(B "/u/fifo", S_IFIFO | 0600, 0, 0), 0);
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

static int body(void) {
    expect("tmpfs", pm_tmpfs(B), 0);
    syscall3(SYS_chmod, B, 0777, 0);
    pm_mkdirs(B "/u");
    syscall3(SYS_chmod, B "/u", 0777, 0);
    pm_put(B, "/a", "file-a", 0644);
    pm_put(B, "/b", "file-b", 0644);
    pm_put(B, "/c", "file-c", 0600);
    pm_mkdirs(B "/dir");
    pm_put(B, "/dir/in", "dir-content", 0644);
    pm_put(B, "/u/src", "user-file", 0666);
    syscall3(SYS_chmod, B "/u/src", 0666, 0);

    expect("whiteout_rename", ren2(B "/a", B "/a2", RENAME_WHITEOUT), 0);
    pm_expect_file("moved_content", B "/a2", "file-a");
    expect("old_name_is_whiteout", is_whiteout(B "/a"), 1);
    expect("moved_mode", pm_mode(B "/a2"), S_IFREG | 0644);

    expect("over_existing", ren2(B "/b", B "/c", RENAME_WHITEOUT), 0);
    pm_expect_file("over_content", B "/c", "file-b");
    expect("over_mode_is_source", pm_mode(B "/c"), S_IFREG | 0644);
    expect("over_old_whiteout", is_whiteout(B "/b"), 1);

    expect("noreplace_exists", ren2(B "/a2", B "/c", RENAME_WHITEOUT | RENAME_NOREPLACE), -EEXIST);
    expect("noreplace_source_kept", pm_mode(B "/a2"), S_IFREG | 0644);
    expect("noreplace_ok", ren2(B "/a2", B "/d", RENAME_WHITEOUT | RENAME_NOREPLACE), 0);
    expect("noreplace_whiteout", is_whiteout(B "/a2"), 1);
    expect("exchange_einval", ren2(B "/c", B "/d", RENAME_WHITEOUT | RENAME_EXCHANGE), -EINVAL);
    expect("exchange_unchanged", pm_file_is(B "/c", "file-b"), 1);
    expect("source_missing", ren2(B "/nothing", B "/x", RENAME_WHITEOUT), -ENOENT);

    expect("rename_dir", ren2(B "/dir", B "/dir2", RENAME_WHITEOUT), 0);
    pm_expect_file("dir_content", B "/dir2/in", "dir-content");
    expect("dir_old_whiteout", is_whiteout(B "/dir"), 1);
    expect("dir_new_is_dir", pm_mode(B "/dir2") & S_IFMT, S_IFDIR);

    /* the whiteout node itself */
    expect("whiteout_open_enxio", a_open(B "/a", O_RDONLY, 0), -ENXIO);
    expect("whiteout_rename_plain", ren2(B "/a", B "/a_moved", 0), 0);
    expect("whiteout_moved", is_whiteout(B "/a_moved"), 1);
    expect("whiteout_unlink", a_unlink(B "/a_moved"), 0);
    expect("whiteout_gone", pm_exists(B "/a_moved"), 0);
    expect("whiteout_name_reusable", a_make_file(B "/a", "again", 0644), 0);
    expect("whiteout_replaced", pm_mode(B "/a"), S_IFREG | 0644);

    /* the same as a normal user */
    a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, as_user());
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    passed += a_shared()[0];
    failed += a_shared()[1];
    expect("user_child_exit_0", st, 0);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    a_rmdir(B);
    pm_in_ns(body);
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
