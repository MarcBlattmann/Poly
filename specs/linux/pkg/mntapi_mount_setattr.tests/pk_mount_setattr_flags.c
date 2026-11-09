/* mount_setattr(dfd, path, flags, &attr, size) changes the options of the
 * mount at path (its top directory) with struct mount_attr { u64 attr_set,
 * attr_clr, propagation, userns_fd } (32 bytes). Bits in attr_clr are
 * cleared first, then bits in attr_set are set, so a bit in both ends up
 * set. RDONLY 0x01 (EROFS on creating a file), NOSUID 0x02, NODEV 0x04
 * (opening a device node is EACCES), NOEXEC 0x08 (execve is EACCES) and
 * NODIRATIME 0x80 are independent bits; the atime mode is the three bits
 * 0x70: RELATIME 0, NOATIME 0x10, STRICTATIME 0x20, and to change it
 * attr_clr must contain all of MOUNT_ATTR__ATIME (0x70) whenever attr_set has
 * a non-zero atime value (EINVAL otherwise; two values at once are EINVAL
 * too; clearing the bits alone gives RELATIME); NOSYMFOLLOW 0x200000 makes a path through a symbolic link
 * ELOOP. The result is visible in statfs f_flags (RDONLY 1, NOSUID 2,
 * NODEV 4, NOEXEC 8, NOATIME 0x400, NODIRATIME 0x800, RELATIME 0x1000,
 * NOSYMFOLLOW 0x2000; the kernel's 0x20 bit is masked out here). Bits
 * outside the known set, in attr_set or attr_clr, are EINVAL and change
 * nothing. Only this mount changes, not other mounts of the same filesystem.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_mount_setattr_flags"

static const char *g_t;

static i64 setattr(u64 set, u64 clr) {
    struct pm_mount_attr a = {set, clr, 0, 0};
    return pm_mount_setattr(AT_FDCWD, g_t, 0, &a, sizeof a);
}

static void flags_are(const char *name, i64 want) { check(name, pm_mntflags(g_t) == want, pm_mntflags(g_t)); }

static void body(const char *base) {
    char t[128], other[128];
    pm_path(t, base, "t");
    g_t = t;
    expect("mkdir_t", pm_mkdir(t), 0);
    expect("mount_t", pm_mount("t", t, "tmpfs", 0, "size=1m"), 0);
    pm_path(other, base, "other");
    expect("mkdir_other", pm_mkdir(other), 0);
    expect("bind_other", pm_mount(t, other, 0, MS_BIND, 0), 0);
    flags_are("initial_flags", ST_RELATIME);

    expect("rdonly_set", setattr(MOUNT_ATTR_RDONLY, 0), 0);
    flags_are("rdonly_flags", ST_RELATIME | ST_RDONLY);
    expect("rdonly_erofs", pm_can_create(t), -EROFS);
    expect("other_bind_unchanged", pm_can_create(other), 0);
    expect("rdonly_clear", setattr(0, MOUNT_ATTR_RDONLY), 0);
    flags_are("rdonly_cleared_flags", ST_RELATIME);
    expect("writable_again", pm_can_create(t), 0);

    expect("nosuid_nodev_noexec", setattr(MOUNT_ATTR_NOSUID | MOUNT_ATTR_NODEV | MOUNT_ATTR_NOEXEC, 0), 0);
    flags_are("three_flags", ST_RELATIME | ST_NOSUID | ST_NODEV | ST_NOEXEC);
    expect("clear_nodev", setattr(0, MOUNT_ATTR_NODEV), 0);
    flags_are("nodev_cleared", ST_RELATIME | ST_NOSUID | ST_NOEXEC);
    expect("clear_all", setattr(0, MOUNT_ATTR_NOSUID | MOUNT_ATTR_NOEXEC), 0);
    flags_are("all_cleared", ST_RELATIME);

    /* effects of NOEXEC and NODEV */
    char px[200], pn[200];
    expect("exec_file", a_make_file(pm_path(px, t, "prog"), "not a program\n", 0755), 0);
    expect("mknod_null", syscall4(SYS_mknodat, AT_FDCWD, pm_path(pn, t, "null"), S_IFCHR | 0666, (1 << 8) | 3), 0);
    char *argv[] = {px, 0};
    char *envp[] = {0};
    expect("exec_plain", syscall3(SYS_execve, px, argv, envp), -ENOEXEC);
    check("open_dev_plain", a_open(pn, O_RDONLY, 0) >= 0, 0);
    expect("set_noexec_nodev", setattr(MOUNT_ATTR_NOEXEC | MOUNT_ATTR_NODEV, 0), 0);
    expect("exec_noexec", syscall3(SYS_execve, px, argv, envp), -EACCES);
    expect("open_dev_nodev", a_open(pn, O_RDONLY, 0), -EACCES);
    expect("clear_noexec_nodev", setattr(0, MOUNT_ATTR_NOEXEC | MOUNT_ATTR_NODEV), 0);
    expect("exec_plain_again", syscall3(SYS_execve, px, argv, envp), -ENOEXEC);

    /* atime modes */
    expect("noatime", setattr(MOUNT_ATTR_NOATIME, MOUNT_ATTR__ATIME), 0);
    flags_are("noatime_flags", ST_NOATIME);
    expect("strictatime", setattr(MOUNT_ATTR_STRICTATIME, MOUNT_ATTR__ATIME), 0);
    flags_are("strictatime_flags", 0);
    expect("relatime", setattr(MOUNT_ATTR_RELATIME, MOUNT_ATTR__ATIME), 0);
    flags_are("relatime_flags", ST_RELATIME);
    expect("clear_atime_only", setattr(0, MOUNT_ATTR__ATIME), 0);
    flags_are("clear_atime_means_relatime", ST_RELATIME);
    expect("noatime_without_clear", setattr(MOUNT_ATTR_NOATIME, 0), -EINVAL);
    expect("strictatime_without_clear", setattr(MOUNT_ATTR_STRICTATIME, 0), -EINVAL);
    expect("noatime_with_partial_clear", setattr(MOUNT_ATTR_NOATIME, 0x10), -EINVAL);
    flags_are("unchanged_after_errors", ST_RELATIME);
    expect("two_atime_values", setattr(MOUNT_ATTR_NOATIME | MOUNT_ATTR_STRICTATIME, MOUNT_ATTR__ATIME), -EINVAL);
    expect("atime_value_0x40", setattr(0x40, MOUNT_ATTR__ATIME), -EINVAL);
    expect("nodiratime", setattr(MOUNT_ATTR_NODIRATIME, 0), 0);
    flags_are("nodiratime_flags", ST_RELATIME | ST_NODIRATIME);
    expect("clear_nodiratime", setattr(0, MOUNT_ATTR_NODIRATIME), 0);
    flags_are("nodiratime_cleared", ST_RELATIME);

    /* NOSYMFOLLOW */
    char tgt[200], lnk[200];
    expect("target", a_make_file(pm_path(tgt, t, "target"), "x", 0644), 0);
    expect("link", syscall3(SYS_symlinkat, "target", AT_FDCWD, pm_path(lnk, t, "link")), 0);
    check("link_followed", a_open(lnk, O_RDONLY, 0) >= 0, 0);
    expect("nosymfollow", setattr(MOUNT_ATTR_NOSYMFOLLOW, 0), 0);
    flags_are("nosymfollow_flags", ST_RELATIME | ST_NOSYMFOLLOW);
    expect("link_refused", a_open(lnk, O_RDONLY, 0), -ELOOP);
    expect("direct_open_ok", a_open(tgt, O_RDONLY, 0) >= 0 ? 0 : -1, 0);
    expect("clear_nosymfollow", setattr(0, MOUNT_ATTR_NOSYMFOLLOW), 0);
    check("link_followed_again", a_open(lnk, O_RDONLY, 0) >= 0, 0);

    /* set and clear together, no-ops, repeats */
    expect("overlap_set_wins", setattr(MOUNT_ATTR_RDONLY, MOUNT_ATTR_RDONLY), 0);
    flags_are("overlap_flags", ST_RELATIME | ST_RDONLY);
    expect("overlap_other_way", setattr(MOUNT_ATTR_NOSUID, MOUNT_ATTR_RDONLY | MOUNT_ATTR_NOSUID), 0);
    flags_are("overlap2_flags", ST_RELATIME | ST_NOSUID);
    expect("set_twice", setattr(MOUNT_ATTR_NOSUID, 0), 0);
    expect("zero_attr", setattr(0, 0), 0);
    flags_are("zero_changes_nothing", ST_RELATIME | ST_NOSUID);
    expect("all_at_once", setattr(MOUNT_ATTR_RDONLY | MOUNT_ATTR_NODEV | MOUNT_ATTR_NOEXEC | MOUNT_ATTR_NOATIME |
                                      MOUNT_ATTR_NODIRATIME | MOUNT_ATTR_NOSYMFOLLOW,
                                  MOUNT_ATTR__ATIME),
           0);
    flags_are("all_flags", ST_RDONLY | ST_NOSUID | ST_NODEV | ST_NOEXEC | ST_NOATIME | ST_NODIRATIME |
                               ST_NOSYMFOLLOW);

    /* unknown bits */
    expect("set_bit_0x400", setattr(0x400, 0), -EINVAL);
    expect("set_bit_31", setattr(0x80000000UL, 0), -EINVAL);
    expect("set_bit_32", setattr(0x100000000UL, 0), -EINVAL);
    expect("clear_bit_31", setattr(0, 0x80000000UL), -EINVAL);
    expect("clear_bit_63", setattr(0, 1UL << 63), -EINVAL);
    flags_are("unchanged_by_unknown_bits", ST_RDONLY | ST_NOSUID | ST_NODEV | ST_NOEXEC | ST_NOATIME |
                                               ST_NODIRATIME | ST_NOSYMFOLLOW);
    expect("other_bind_still_plain", pm_mntflags(other), ST_RELATIME);
}

int test_main(void) { return pm_main(BASE, body); }
