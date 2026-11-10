/* mount_setattr: the order in which the arguments are judged, and the
 * rules for MOUNT_ATTR_IDMAP's userns_fd.
 * Order: (1) flags with a bit outside AT_SYMLINK_NOFOLLOW, AT_NO_AUTOMOUNT,
 * AT_EMPTY_PATH, AT_RECURSIVE: EINVAL; (2) size above 4096: E2BIG; size
 * below 32: EINVAL; (3) the caller's privilege: without CAP_SYS_ADMIN
 * EPERM; (4) the struct is read: a NULL or bad pointer is EFAULT, a
 * non-zero byte after the first 32 is E2BIG; (5) its contents: unknown
 * attribute bits, a bad atime or propagation value and the MOUNT_ATTR_IDMAP
 * rules; (6) only then the path: EBADF for a bad dfd, ENOENT, and EINVAL
 * for what is not the top of a mount. So an unprivileged caller sees
 * EINVAL or E2BIG for bad flags and sizes and EPERM for everything else,
 * whatever the attr pointer, the attributes and the path are; and a
 * privileged caller sees the error of the struct before the error of the
 * path. A call that fails changes nothing.
 * MOUNT_ATTR_IDMAP (0x100000) in attr_set takes userns_fd: a number that
 * is not an open descriptor is EBADF; a descriptor that is not a user
 * namespace file (a regular file, a directory, another kind of namespace)
 * is EINVAL; a value above 2147483647 is EINVAL; MOUNT_ATTR_IDMAP in
 * attr_clr is EINVAL also when userns_fd is given.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_mount_setattr_checks"

struct call {
    const char *path;
    u64 flags;
    const void *attr;
    u64 size;
    i64 dfd;
};

static i64 do_call(void *arg) {
    struct call *c = arg;
    return pm_mount_setattr(c->dfd, c->path, c->flags, c->attr, c->size);
}

static i64 nobody(const char *path, u64 flags, const void *attr, u64 size) {
    struct call c = {path, flags, attr, size, AT_FDCWD};
    return pm_as_nobody(do_call, &c);
}

struct big {
    struct pm_mount_attr a;
    u64 more[8];
};

static void body(const char *base) {
    char t[128], d[160], q[200];
    pm_path(t, base, "m");
    expect("mkdir", pm_mkdir(t), 0);
    expect("mount", pm_mount("m", t, "tmpfs", 0, "size=1m"), 0);
    expect("mkdir_inside", pm_mkdir(pm_path(d, t, "inside")), 0);
    pm_path(q, t, "absent");

    struct pm_mount_attr ok = {MOUNT_ATTR_NODEV, 0, 0, 0};
    struct pm_mount_attr badbits = {0x4000, 0, 0, 0};
    struct pm_mount_attr badprop = {0, 0, 0x7, 0};
    struct big tail;
    memset(&tail, 0, sizeof tail);
    tail.more[1] = 5;

    /* without CAP_SYS_ADMIN */
    expect("user_bad_flag", nobody(t, 0x4, &ok, 32), -EINVAL);
    expect("user_bad_flag_null_attr", nobody(q, 0x10, 0, 32), -EINVAL);
    expect("user_size_16", nobody(t, 0, &ok, 16), -EINVAL);
    expect("user_size_0_null_attr", nobody(t, 0, 0, 0), -EINVAL);
    expect("user_size_5000", nobody(t, 0, &ok, 5000), -E2BIG);
    expect("user_bad_flag_before_big_size", nobody(t, 0x4, &ok, 5000), -EINVAL);
    expect("user_valid_call", nobody(t, 0, &ok, 32), -EPERM);
    expect("user_null_attr", nobody(t, 0, 0, 32), -EPERM);
    expect("user_bad_attr_pointer", nobody(t, 0, (const void *)8, 32), -EPERM);
    expect("user_nonzero_tail", nobody(t, 0, &tail, 48), -EPERM);
    expect("user_unknown_attr_bits", nobody(t, 0, &badbits, 32), -EPERM);
    expect("user_missing_path", nobody(q, 0, &ok, 32), -EPERM);
    expect("user_not_mount_top", nobody(d, AT_RECURSIVE, &ok, 32), -EPERM);
    expect("user_empty_path", nobody("", 0, &ok, 32), -EPERM);
    expect("nothing_changed_by_user", pm_mntflags(t) & ST_NODEV, 0);

    /* with CAP_SYS_ADMIN: flags, then size, then the struct, then the path */
    expect("root_bad_flag_and_small_size", pm_mount_setattr(AT_FDCWD, q, 0x4, 0, 8), -EINVAL);
    expect("root_small_size_null_attr", pm_mount_setattr(AT_FDCWD, q, 0, 0, 8), -EINVAL);
    expect("root_big_size_null_attr", pm_mount_setattr(AT_FDCWD, q, 0, 0, 5000), -E2BIG);
    expect("root_null_attr_missing_path", pm_mount_setattr(AT_FDCWD, q, 0, 0, 32), -EFAULT);
    expect("root_tail_missing_path", pm_mount_setattr(AT_FDCWD, q, 0, &tail, 48), -E2BIG);
    expect("root_bad_bits_missing_path", pm_mount_setattr(AT_FDCWD, q, 0, &badbits, 32), -EINVAL);
    expect("root_bad_propagation_missing_path", pm_mount_setattr(AT_FDCWD, q, 0, &badprop, 32), -EINVAL);
    expect("root_bad_bits_bad_dfd", pm_mount_setattr(997, "m", 0, &badbits, 32), -EINVAL);
    expect("root_good_struct_bad_dfd", pm_mount_setattr(997, "m", 0, &ok, 32), -EBADF);
    expect("root_good_struct_missing_path", pm_mount_setattr(AT_FDCWD, q, 0, &ok, 32), -ENOENT);
    expect("root_good_struct_not_top", pm_mount_setattr(AT_FDCWD, d, 0, &ok, 32), -EINVAL);
    expect("nothing_changed_by_errors", pm_mntflags(t) & ST_NODEV, 0);

    /* MOUNT_ATTR_IDMAP: userns_fd */
    struct pm_mount_attr idm = {MOUNT_ATTR_IDMAP, 0, 0, 0};
    i64 m = pm_tmpfs_fd("1m", 0, FSMOUNT_CLOEXEC);
    check("detached_mount", m >= 0, m);
    idm.userns_fd = 995;
    expect("idmap_fd_closed_number", pm_mount_setattr(m, "", AT_EMPTY_PATH, &idm, sizeof idm), -EBADF);
    expect("idmap_fd_closed_missing_path", pm_mount_setattr(AT_FDCWD, q, 0, &idm, sizeof idm), -EBADF);
    i64 reg = a_open(pm_path(q, t, "regular"), O_RDWR | O_CREAT, 0644);
    check("regular_file", reg >= 0, reg);
    idm.userns_fd = (u64)reg;
    expect("idmap_fd_regular_file", pm_mount_setattr(m, "", AT_EMPTY_PATH, &idm, sizeof idm), -EINVAL);
    i64 dirfd = a_open(t, O_RDONLY | O_DIRECTORY, 0);
    idm.userns_fd = (u64)dirfd;
    expect("idmap_fd_directory", pm_mount_setattr(m, "", AT_EMPTY_PATH, &idm, sizeof idm), -EINVAL);
    i64 utsfd = a_open("/proc/self/ns/uts", O_RDONLY, 0);
    check("uts_ns_file", utsfd >= 0, utsfd);
    idm.userns_fd = (u64)utsfd;
    expect("idmap_fd_uts_namespace", pm_mount_setattr(m, "", AT_EMPTY_PATH, &idm, sizeof idm), -EINVAL);
    idm.userns_fd = 0x80000000UL;
    expect("idmap_fd_above_int_max", pm_mount_setattr(m, "", AT_EMPTY_PATH, &idm, sizeof idm), -EINVAL);
    idm.userns_fd = 0x100000000UL + (u64)utsfd;
    expect("idmap_fd_high_bits", pm_mount_setattr(m, "", AT_EMPTY_PATH, &idm, sizeof idm), -EINVAL);
    struct pm_mount_attr clr = {0, MOUNT_ATTR_IDMAP, 0, (u64)utsfd};
    expect("idmap_in_clr_with_fd", pm_mount_setattr(m, "", AT_EMPTY_PATH, &clr, sizeof clr), -EINVAL);
    struct pm_mount_attr both = {MOUNT_ATTR_IDMAP, MOUNT_ATTR_IDMAP, 0, 995};
    expect("idmap_in_set_and_clr", pm_mount_setattr(m, "", AT_EMPTY_PATH, &both, sizeof both), -EINVAL);
    struct pm_mount_attr plain = {MOUNT_ATTR_NOEXEC, 0, 0, 995};
    expect("fd_field_ignored_without_idmap", pm_mount_setattr(m, "", AT_EMPTY_PATH, &plain, sizeof plain), 0);
}

int test_main(void) { return pm_main(BASE, body); }
