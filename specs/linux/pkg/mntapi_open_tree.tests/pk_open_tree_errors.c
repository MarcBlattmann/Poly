/* open_tree(..., OPEN_TREE_CLONE) errors and permissions. Cloning needs
 * CAP_SYS_ADMIN (EPERM first, even for a missing path); the same call
 * without OPEN_TREE_CLONE does not. Missing path ENOENT (also ""), NULL
 * pointer EFAULT, unknown flag bits EINVAL, relative path with an invalid
 * dfd EBADF. With AT_EMPTY_PATH the dfd itself is cloned (a directory
 * descriptor of a plain directory gives a clone of that directory). A
 * descriptor returned by open_tree(OPEN_TREE_CLONE) can be used for
 * openat/fstat before it is attached. In a user and mount namespace of a
 * normal user, cloning works for the mounts that user can see.
 * poly-diff: needs=root (the test drops to uid 65534 itself)
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_open_tree_errors"
#define USERROOT "/tmp/pk_open_tree_errors_u"
#define PR_SET_DUMPABLE 4

static i64 nobody_clone(void *arg) {
    i64 r = pm_open_tree(AT_FDCWD, arg, OPEN_TREE_CLONE);
    return r >= 0 ? 0 : r;
}
static i64 nobody_clone_missing(void *arg) {
    (void)arg;
    return pm_open_tree(AT_FDCWD, "/nonexistent-for-open-tree", OPEN_TREE_CLONE);
}
static i64 nobody_plain_missing(void *arg) {
    (void)arg;
    return pm_open_tree(AT_FDCWD, "/nonexistent-for-open-tree", 0);
}

static void write_file(const char *path, const char *text, const char *name) {
    i64 fd = a_open(path, O_WRONLY, 0);
    i64 n = fd < 0 ? fd : a_write(fd, text, str_len(text));
    check(name, n == (i64)str_len(text), n);
    if (fd >= 0) a_close(fd);
}

static void user_part(void *arg) {
    (void)arg;
    syscall2(SYS_prctl, PR_SET_DUMPABLE, 1);
    expect("eperm_before_unshare", pm_open_tree(AT_FDCWD, USERROOT, OPEN_TREE_CLONE), -EPERM);
    expect("unshare_user_mount", syscall1(SYS_unshare, CLONE_NEWUSER | CLONE_NEWNS), 0);
    write_file("/proc/self/setgroups", "deny", "setgroups_deny");
    write_file("/proc/self/uid_map", "0 65534 1\n", "uid_map");
    write_file("/proc/self/gid_map", "0 65534 1\n", "gid_map");
    expect("make_private", pm_mount(0, "/", 0, MS_REC | MS_PRIVATE, 0), 0);
    expect("mount_tmpfs", pm_mount("u", USERROOT, "tmpfs", 0, "size=1m"), 0);
    expect("file", pm_touch(USERROOT "/f"), 0);
    i64 c = pm_open_tree(AT_FDCWD, USERROOT, OPEN_TREE_CLONE | OPEN_TREE_CLOEXEC);
    check("clone_in_userns", c >= 3, c);
    expect("mkdir_target", pm_mkdir(USERROOT "/t"), 0);
    expect("attach_clone", pm_attach(c, USERROOT "/t"), 0);
    expect("clone_shows_file", pm_exists(USERROOT "/t/f"), 0);
}

static void body(const char *base) {
    char t[128], d[160], f[160], q[200];
    pm_path(t, base, "t");
    expect("mkdir_t", pm_mkdir(t), 0);
    expect("mount_t", pm_mount("t", t, "tmpfs", 0, "size=1m"), 0);
    expect("dir_d", pm_mkdir(pm_path(d, t, "d")), 0);
    expect("file_f", pm_touch(pm_path(f, t, "f")), 0);

    expect("nobody_clone_eperm", pm_as_nobody(nobody_clone, t), -EPERM);
    expect("nobody_clone_missing_eperm", pm_as_nobody(nobody_clone_missing, 0), -EPERM);
    expect("nobody_plain_missing_enoent", pm_as_nobody(nobody_plain_missing, 0), -ENOENT);

    expect("clone_missing", pm_open_tree(AT_FDCWD, pm_path(q, t, "none"), OPEN_TREE_CLONE), -ENOENT);
    expect("clone_empty_name", pm_open_tree(AT_FDCWD, "", OPEN_TREE_CLONE), -ENOENT);
    expect("clone_null", pm_open_tree(AT_FDCWD, 0, OPEN_TREE_CLONE), -EFAULT);
    expect("clone_bad_flag", pm_open_tree(AT_FDCWD, t, OPEN_TREE_CLONE | 0x40000000), -EINVAL);
    expect("clone_bad_flag_2", pm_open_tree(AT_FDCWD, t, OPEN_TREE_CLONE | 2), -EINVAL);
    expect("clone_relative_bad_dfd", pm_open_tree(996, "x", OPEN_TREE_CLONE), -EBADF);
    expect("clone_empty_path_bad_dfd", pm_open_tree(996, "", OPEN_TREE_CLONE | AT_EMPTY_PATH), -EBADF);
    expect("clone_empty_path_minus_one", pm_open_tree(-1, "", OPEN_TREE_CLONE | AT_EMPTY_PATH), -EBADF);

    /* AT_EMPTY_PATH: the dfd itself */
    i64 dfd = a_open(d, O_RDONLY | O_DIRECTORY, 0);
    check("open_d", dfd >= 0, dfd);
    expect("make_in_d", pm_touch(pm_path(q, d, "inside")), 0);
    i64 c = pm_open_tree(dfd, "", OPEN_TREE_CLONE | AT_EMPTY_PATH | OPEN_TREE_CLOEXEC);
    check("clone_of_dirfd", c >= 3, c);
    struct a_stat st;
    expect("clone_has_inside", syscall4(SYS_newfstatat, c, "inside", &st, 0), 0);
    pm_path(q, base, "att");
    expect("mkdir_att", pm_mkdir(q), 0);
    expect("attach", pm_attach(c, q), 0);
    char q2[200];
    expect("attached_has_inside", pm_exists(pm_path(q2, q, "inside")), 0);
    a_close(c);

    /* relative path with a directory descriptor */
    i64 tfd = a_open(t, O_RDONLY | O_DIRECTORY, 0);
    c = pm_open_tree(tfd, "d", OPEN_TREE_CLONE);
    check("clone_relative_to_dirfd", c >= 3, c);
    expect("relative_clone_has_inside", syscall4(SYS_newfstatat, c, "inside", &st, 0), 0);

    /* user namespace */
    expect("mkdir_userroot", pm_mkdir(USERROOT), 0);
    pm_child(user_part, 0, 1);
}

int test_main(void) {
    int r = pm_main(BASE, body);
    a_rmdir(USERROOT);
    return r;
}
