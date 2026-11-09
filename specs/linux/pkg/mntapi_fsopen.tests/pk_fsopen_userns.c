/* fsopen in a user namespace: a normal user who made a user namespace and a
 * mount namespace (and is root there) may open contexts for the filesystem
 * types that allow mounting from a user namespace (tmpfs, proc, devpts,
 * overlay, fuse), and may create and mount a tmpfs with fsmount and
 * move_mount. Outside the namespace the same user gets EPERM for fsopen.
 * poly-diff: needs=root (the test drops to uid 65534 itself)
 * poly-diff: syscalls=off (several processes)
 */
#include "p_mntapi.h"

#define ROOT "/tmp/pk_fsopen_userns"
#define PR_SET_DUMPABLE 4

static void write_file(const char *path, const char *text, const char *name) {
    i64 fd = a_open(path, O_WRONLY, 0);
    i64 n = fd < 0 ? fd : a_write(fd, text, str_len(text));
    check(name, n == (i64)str_len(text), n);
    if (fd >= 0) a_close(fd);
}

static void user_part(void *arg) {
    (void)arg;
    syscall2(SYS_prctl, PR_SET_DUMPABLE, 1);
    expect("eperm_before_unshare", pm_fsopen("tmpfs", 0), -EPERM);
    expect("unshare_user_mount", syscall1(SYS_unshare, CLONE_NEWUSER | CLONE_NEWNS), 0);
    write_file("/proc/self/setgroups", "deny", "setgroups_deny");
    write_file("/proc/self/uid_map", "0 65534 1\n", "uid_map");
    write_file("/proc/self/gid_map", "0 65534 1\n", "gid_map");
    expect("make_private", pm_mount(0, "/", 0, MS_REC | MS_PRIVATE, 0), 0);
    const char *ok[] = {"tmpfs", "proc", "devpts", "overlay", "fuse"};
    for (int i = 0; i < 5; i++) {
        i64 fd = pm_fsopen(ok[i], FSOPEN_CLOEXEC);
        check(ok[i], fd >= 3, fd);
        if (fd >= 0) a_close(fd);
    }
    expect("unknown_type_still_enodev", pm_fsopen("nonexistent", 0), -ENODEV);
    i64 fc = pm_fsopen("tmpfs", FSOPEN_CLOEXEC);
    expect("create_tmpfs", pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0), 0);
    i64 m = pm_fsmount(fc, FSMOUNT_CLOEXEC, 0);
    check("fsmount", m >= 0, m);
    expect("attach", pm_attach(m, ROOT), 0);
    expect("file_in_userns_tmpfs", pm_touch(ROOT "/f"), 0);
}

int test_main(void) {
    a_block_sigchld();
    expect("mkdir_root_dir", pm_mkdir(ROOT), 0);
    pm_child(user_part, 0, 1);
    expect("rmdir_root_dir", a_rmdir(ROOT), 0);
    return finish();
}
