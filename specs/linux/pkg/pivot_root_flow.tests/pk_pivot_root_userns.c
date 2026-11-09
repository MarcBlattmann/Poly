/* Rootless pivot_root: a normal user (uid 65534, no capabilities) makes a
 * user and mount namespace (unshare(CLONE_NEWUSER | CLONE_NEWNS)), maps
 * itself to root there, mounts a tmpfs and runs the container sequence:
 * private "/", chdir into the tmpfs, pivot_root(".", "."), umount2(".",
 * MNT_DETACH). The call is allowed because the caller has CAP_SYS_ADMIN in
 * the user namespace that owns the mount namespace. Before unshare the same
 * call is EPERM.
 * poly-diff: needs=root (the test drops to uid 65534 itself)
 * poly-diff: syscalls=off (several processes)
 */
#include "p_mntapi.h"

#define ROOT "/tmp/pk_pivot_root_userns"
#define SYS_prctl_ 157
#define PR_SET_DUMPABLE 4

static void write_file(const char *path, const char *text, const char *name) {
    i64 fd = a_open(path, O_WRONLY, 0);
    i64 n = fd < 0 ? fd : a_write(fd, text, str_len(text));
    check(name, n == (i64)str_len(text), n);
    if (fd >= 0) a_close(fd);
}

static void user_part(void *arg) {
    (void)arg;
    syscall2(SYS_prctl_, PR_SET_DUMPABLE, 1);
    expect("eperm_before_unshare", pm_pivot_root(ROOT, ROOT), -EPERM);
    expect("mkdir_root", pm_mkdir(ROOT), 0);
    expect("unshare_user_mount", syscall1(SYS_unshare, CLONE_NEWUSER | CLONE_NEWNS), 0);
    write_file("/proc/self/setgroups", "deny", "setgroups_deny");
    write_file("/proc/self/uid_map", "0 65534 1\n", "uid_map");
    write_file("/proc/self/gid_map", "0 65534 1\n", "gid_map");
    expect("make_private", pm_mount(0, "/", 0, MS_REC | MS_PRIVATE, 0), 0);
    expect("mount_tmpfs", pm_mount("u", ROOT, "tmpfs", 0, 0), 0);
    expect("mkdir_old", pm_mkdir(ROOT "/old"), 0);
    expect("file_f", pm_touch(ROOT "/f"), 0);
    expect("chdir_new_root", syscall1(SYS_chdir, ROOT), 0);
    expect("pivot_dot_dot", pm_pivot_root(".", "."), 0);
    expect("detach_old", pm_umount2(".", MNT_DETACH), 0);
    expect("new_root_visible", pm_exists("/f"), 0);
    expect("old_tmp_gone", pm_exists("/tmp"), -ENOENT);
}

int test_main(void) {
    a_block_sigchld();
    expect("mkdir_root_dir", pm_mkdir(ROOT), 0);
    pm_child(user_part, 0, 1);
    expect("rmdir_root_dir", a_rmdir(ROOT), 0);
    return finish();
}
