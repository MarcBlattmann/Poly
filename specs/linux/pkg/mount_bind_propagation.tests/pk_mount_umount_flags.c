/* umount2 on mounts of tmpfs: flags 0 unmounts the top mount at the path
 * (EINVAL when the path is not a mount point, ENOENT when it does not exist,
 * EBUSY when a file is open on it, the working directory is on it, or
 * another mount is below it). MNT_FORCE behaves like flag 0 here. MNT_DETACH
 * removes the mount from the namespace at once even if busy; an open file
 * keeps working. MNT_EXPIRE: with MNT_FORCE or MNT_DETACH it is EINVAL; the
 * first call on an unused mount marks it and returns EAGAIN, the second
 * unmounts it, and use in between (opening a file there) resets the mark;
 * on a busy mount it is EBUSY. UMOUNT_NOFOLLOW: a symbolic link to a mount
 * point is not followed (EINVAL), without the flag it is followed. Unknown
 * flag bits are EINVAL. A normal user gets EPERM.
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_muf"
#define SYS_setresuid_ 117
#define SYS_setresgid_ 119

static int as_user(void) {
    syscall3(SYS_setresgid_, 65534, 65534, 65534);
    syscall3(SYS_setresuid_, 65534, 65534, 65534);
    expect("user_umount_eperm", pm_umount(B "/m1", 0), -EPERM);
    expect("user_umount_detach_eperm", pm_umount(B "/m1", MNT_DETACH), -EPERM);
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

static int body(void) {
    expect("tmpfs", pm_tmpfs(B), 0);
    pm_mkdirs(B "/m1");
    pm_mkdirs(B "/m2");
    pm_mkdirs(B "/m3");
    pm_mkdirs(B "/plain");
    syscall3(SYS_symlink, B "/m3", B "/link", 0);

    expect("mount1", pm_mount("pk1", B "/m1", "tmpfs", 0, 0), 0);
    syscall3(SYS_chmod, B, 0777, 0);
    syscall3(SYS_chmod, B "/m1", 0777, 0);
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, as_user());
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    passed += a_shared()[0];
    failed += a_shared()[1];
    expect("user_child_exit_0", st, 0);
    pm_put(B, "/m1/f", "x", 0644);

    /* errors that do not depend on busy mounts */
    expect("not_a_mountpoint", pm_umount(B "/plain", 0), -EINVAL);
    expect("missing_path", pm_umount(B "/nothing", 0), -ENOENT);
    expect("unknown_flag", pm_umount(B "/m1", 0x80), -EINVAL);
    expect("force_expire", pm_umount(B "/m1", MNT_FORCE | MNT_EXPIRE), -EINVAL);
    expect("detach_expire", pm_umount(B "/m1", MNT_DETACH | MNT_EXPIRE), -EINVAL);
    pm_expect_file("still_mounted", B "/m1/f", "x");

    /* busy: open file */
    i64 fd = a_open(B "/m1/f", O_RDONLY, 0);
    check("open", fd >= 0, fd);
    expect("busy_open_file", pm_umount(B "/m1", 0), -EBUSY);
    expect("busy_expire", pm_umount(B "/m1", MNT_EXPIRE), -EBUSY);
    expect("busy_force", pm_umount(B "/m1", MNT_FORCE), -EBUSY);
    expect("detach_busy", pm_umount(B "/m1", MNT_DETACH), 0);
    expect("detached_path_gone", pm_exists(B "/m1/f"), 0);
    char buf[4];
    memset(buf, 0, sizeof buf);
    expect("detached_read", a_read(fd, buf, 1), 1);
    check("detached_content", buf[0] == 'x', buf[0]);
    a_close(fd);

    /* busy: working directory */
    expect("mount2", pm_mount("pk2", B "/m2", "tmpfs", 0, 0), 0);
    expect("chdir_in", syscall1(SYS_chdir, B "/m2"), 0);
    expect("busy_cwd", pm_umount(B "/m2", 0), -EBUSY);
    expect("chdir_out", syscall1(SYS_chdir, "/"), 0);
    expect("umount_after_cwd", pm_umount(B "/m2", 0), 0);

    /* busy: mount below */
    expect("mount2_again", pm_mount("pk2", B "/m2", "tmpfs", 0, 0), 0);
    pm_mkdirs(B "/m2/sub");
    expect("mount_below", pm_mount("pksub", B "/m2/sub", "tmpfs", 0, 0), 0);
    expect("busy_submount", pm_umount(B "/m2", 0), -EBUSY);
    expect("umount_below", pm_umount(B "/m2/sub", 0), 0);
    expect("umount_top", pm_umount(B "/m2", 0), 0);

    /* MNT_EXPIRE on an idle mount */
    expect("mount_expire", pm_mount("pke", B "/m2", "tmpfs", 0, 0), 0);
    expect("expire_first", pm_umount(B "/m2", MNT_EXPIRE), -EAGAIN);
    expect("expire_second", pm_umount(B "/m2", MNT_EXPIRE), 0);
    expect("expired_gone", pm_exists(B "/m2/f"), 0);
    expect("mount_expire2", pm_mount("pke", B "/m2", "tmpfs", 0, 0), 0);
    expect("expire2_first", pm_umount(B "/m2", MNT_EXPIRE), -EAGAIN);
    pm_put(B, "/m2/touched", "t", 0644);
    expect("expire2_after_use", pm_umount(B "/m2", MNT_EXPIRE), -EAGAIN);
    expect("expire2_then", pm_umount(B "/m2", MNT_EXPIRE), 0);

    /* MNT_FORCE on an idle mount is a plain unmount */
    expect("mount_force", pm_mount("pkf", B "/m2", "tmpfs", 0, 0), 0);
    expect("force_idle", pm_umount(B "/m2", MNT_FORCE), 0);

    /* UMOUNT_NOFOLLOW */
    expect("mount3", pm_mount("pk3", B "/m3", "tmpfs", 0, 0), 0);
    expect("nofollow_link", pm_umount(B "/link", UMOUNT_NOFOLLOW), -EINVAL);
    pm_mkdirs(B "/m3/x");
    expect("still_mounted3", pm_exists(B "/m3/x"), 1);
    expect("nofollow_direct", pm_umount(B "/m3", UMOUNT_NOFOLLOW), 0);
    expect("mount3_again", pm_mount("pk3", B "/m3", "tmpfs", 0, 0), 0);
    expect("follow_link", pm_umount(B "/link", 0), 0);
    expect("followed_gone", pm_exists(B "/m3/x"), 0);

    /* stacked mounts: each umount peels the top one */
    pm_mkdirs(B "/m2");
    expect("stack1", pm_mount("s1", B "/m2", "tmpfs", 0, 0), 0);
    pm_put(B, "/m2/one", "1", 0644);
    expect("stack2", pm_mount("s2", B "/m2", "tmpfs", 0, 0), 0);
    pm_put(B, "/m2/two", "2", 0644);
    pm_expect_list("stack_top", B "/m2", "two");
    expect("peel2", pm_umount(B "/m2", 0), 0);
    pm_expect_list("stack_below", B "/m2", "one");
    expect("peel1", pm_umount(B "/m2", 0), 0);
    pm_expect_list("stack_none", B "/m2", "");
    expect("peel_none", pm_umount(B "/m2", 0), -EINVAL);
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
