/* fspick errors and path rules. The path must name the top directory of a
 * mount (a bind mount of a subdirectory counts; a directory inside a mount,
 * a file, or "." of such a directory is EINVAL). Flags other than the four
 * FSPICK_* bits (1 CLOEXEC, 2 SYMLINK_NOFOLLOW, 4 NO_AUTOMOUNT, 8
 * EMPTY_PATH) are EINVAL. A missing name is ENOENT, also the empty name
 * without FSPICK_EMPTY_PATH; with it the dfd itself is used (it must be a
 * mount's top directory: EINVAL otherwise, EBADF if not open, also -1). An
 * absolute path ignores dfd (even an invalid one); a relative path with an
 * invalid dfd is EBADF. A NULL or invalid pointer is EFAULT. Without
 * CAP_SYS_ADMIN the answer is EPERM before any path is looked at. A symbolic
 * link to a mount top is followed, and refused (EINVAL) with
 * FSPICK_SYMLINK_NOFOLLOW.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_fspick_errors"

static i64 nobody_ok(void *arg) {
    return pm_fspick(AT_FDCWD, arg, 0);
}
static i64 nobody_missing(void *arg) {
    (void)arg;
    return pm_fspick(AT_FDCWD, "/nonexistent-for-fspick", 0);
}

static void body(const char *base) {
    char t[128], d[160], f[160], bnd[128], q[160];
    pm_path(t, base, "t");
    expect("mkdir_t", pm_mkdir(t), 0);
    expect("mount_t", pm_mount("t", t, "tmpfs", 0, "size=1m"), 0);
    expect("mkdir_d", pm_mkdir(pm_path(d, t, "d")), 0);
    expect("file_f", pm_touch(pm_path(f, t, "f")), 0);

    i64 r = pm_fspick(AT_FDCWD, t, 0);
    check("mount_top", r >= 3, r);
    a_close(r);
    expect("dir_inside_mount", pm_fspick(AT_FDCWD, d, 0), -EINVAL);
    expect("file", pm_fspick(AT_FDCWD, f, 0), -EINVAL);
    expect("missing", pm_fspick(AT_FDCWD, pm_path(q, t, "none"), 0), -ENOENT);
    expect("empty_name", pm_fspick(AT_FDCWD, "", 0), -ENOENT);
    expect("bad_flag_0x10", pm_fspick(AT_FDCWD, t, 0x10), -EINVAL);
    expect("bad_flag_0x100", pm_fspick(AT_FDCWD, t, 0x100), -EINVAL);
    expect("bad_flags_all", pm_fspick(AT_FDCWD, t, ~0U), -EINVAL);
    expect("null_path", pm_fspick(AT_FDCWD, 0, 0), -EFAULT);
    expect("bad_pointer", pm_fspick(AT_FDCWD, (const char *)1, 0), -EFAULT);

    /* directory descriptors */
    expect("absolute_path_ignores_dfd", (r = pm_fspick(-1, t, 0)) >= 3 ? 0 : r, 0);
    if (r >= 0) a_close(r);
    expect("absolute_path_ignores_bad_dfd", (r = pm_fspick(996, t, 0)) >= 3 ? 0 : r, 0);
    if (r >= 0) a_close(r);
    i64 tfd = a_open(t, O_RDONLY | O_DIRECTORY, 0);
    check("open_t", tfd >= 0, tfd);
    r = pm_fspick(tfd, ".", 0);
    check("relative_dot", r >= 3, r);
    a_close(r);
    expect("relative_d", pm_fspick(tfd, "d", 0), -EINVAL);
    expect("relative_bad_dfd", pm_fspick(996, "d", 0), -EBADF);
    expect("relative_minus_one", pm_fspick(-1, "d", 0), -EBADF);
    r = pm_fspick(tfd, "", FSPICK_EMPTY_PATH);
    check("empty_path_mount_top", r >= 3, r);
    a_close(r);
    i64 dfd = a_open(d, O_RDONLY | O_DIRECTORY, 0);
    expect("empty_path_not_mount_top", pm_fspick(dfd, "", FSPICK_EMPTY_PATH), -EINVAL);
    expect("empty_path_not_open", pm_fspick(996, "", FSPICK_EMPTY_PATH), -EBADF);
    expect("empty_path_minus_one", pm_fspick(-1, "", FSPICK_EMPTY_PATH), -EBADF);
    i64 ffd = a_open(f, O_RDONLY, 0);
    expect("empty_path_file", pm_fspick(ffd, "", FSPICK_EMPTY_PATH), -EINVAL);
    i64 pfd = a_open(t, O_PATH, 0);
    r = pm_fspick(pfd, "", FSPICK_EMPTY_PATH);
    check("empty_path_o_path_fd", r >= 3, r);
    a_close(r);
    expect("empty_flag_with_name", pm_fspick(tfd, "d", FSPICK_EMPTY_PATH), -EINVAL);

    /* a bind mount of a subdirectory is a mount top */
    pm_path(bnd, base, "bnd");
    expect("mkdir_bnd", pm_mkdir(bnd), 0);
    expect("bind_subdir", pm_mount(d, bnd, 0, MS_BIND, 0), 0);
    r = pm_fspick(AT_FDCWD, bnd, 0);
    check("bind_mount_top", r >= 3, r);
    a_close(r);

    /* the flag bits that are allowed */
    r = pm_fspick(AT_FDCWD, t, FSPICK_CLOEXEC | FSPICK_SYMLINK_NOFOLLOW | FSPICK_NO_AUTOMOUNT);
    check("all_flags_but_empty", r >= 3, r);
    a_close(r);

    /* symbolic links */
    char ln[160];
    pm_path(ln, base, "link");
    expect("symlink_to_mount_top", syscall3(SYS_symlinkat, t, AT_FDCWD, ln), 0);
    r = pm_fspick(AT_FDCWD, ln, 0);
    check("symlink_followed", r >= 3, r);
    a_close(r);
    expect("symlink_nofollow", pm_fspick(AT_FDCWD, ln, FSPICK_SYMLINK_NOFOLLOW), -EINVAL);

    /* privilege */
    expect("nobody_valid", pm_as_nobody(nobody_ok, t), -EPERM);
    expect("nobody_missing", pm_as_nobody(nobody_missing, 0), -EPERM);
}

int test_main(void) { return pm_main(BASE, body); }
