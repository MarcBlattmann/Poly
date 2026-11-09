/* fspick in a user namespace. In a user and mount namespace made by a normal
 * user, fspick and CMD_RECONFIGURE work on a tmpfs the user mounted there;
 * fspick of the root mount, whose filesystem was created outside the
 * namespace, succeeds, but its reconfigure is EPERM and changes nothing.
 * poly-diff: needs=root (the test drops to uid 65534 itself)
 * poly-diff: syscalls=off (several processes)
 */
#include "p_mntapi.h"

#define ROOT "/tmp/pk_fspick_userns"
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
    expect("unshare_user_mount", syscall1(SYS_unshare, CLONE_NEWUSER | CLONE_NEWNS), 0);
    write_file("/proc/self/setgroups", "deny", "setgroups_deny");
    write_file("/proc/self/uid_map", "0 65534 1\n", "uid_map");
    write_file("/proc/self/gid_map", "0 65534 1\n", "gid_map");
    expect("make_private", pm_mount(0, "/", 0, MS_REC | MS_PRIVATE, 0), 0);
    expect("mount_tmpfs", pm_mount("u", ROOT, "tmpfs", 0, "size=1m"), 0);
    i64 fc = pm_fspick(AT_FDCWD, ROOT, FSPICK_CLOEXEC);
    check("fspick_own_mount", fc >= 3, fc);
    expect("size", pm_fsconfig(fc, FSCONFIG_SET_STRING, "size", "2m", 0), 0);
    expect("reconfigure_own", pm_fsconfig(fc, FSCONFIG_CMD_RECONFIGURE, 0, 0, 0), 0);
    expect("blocks", pm_statfs_blocks(ROOT), 512);
    a_close(fc);

    /* the root mount belongs to a filesystem made outside */
    i64 other = pm_fspick(AT_FDCWD, "/", FSPICK_CLOEXEC);
    check("fspick_foreign_mount", other >= 3, other);
    expect("foreign_flag_ro", pm_fsconfig(other, FSCONFIG_SET_FLAG, "ro", 0, 0), 0);
    expect("foreign_reconfigure_eperm", pm_fsconfig(other, FSCONFIG_CMD_RECONFIGURE, 0, 0, 0), -EPERM);
    expect("still_writable", pm_can_create(ROOT), 0);
}

int test_main(void) {
    a_block_sigchld();
    expect("mkdir_root_dir", pm_mkdir(ROOT), 0);
    pm_child(user_part, 0, 1);
    expect("rmdir_root_dir", a_rmdir(ROOT), 0);
    return finish();
}
