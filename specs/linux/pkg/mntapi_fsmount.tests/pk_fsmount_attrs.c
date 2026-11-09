/* The attr_flags argument of fsmount sets per-mount options. statfs f_flags
 * of the mount (without the internal 0x20 bit) shows them: no attribute
 * gives ST_RELATIME (0x1000); RDONLY adds ST_RDONLY (1) and makes creating
 * a file EROFS; NOSUID 2; NODEV 4 (opening a device node there is EACCES);
 * NOEXEC 8 (execve of an executable file there is EACCES, elsewhere the
 * same file is ENOEXEC because it is no program); NOATIME replaces relatime
 * by ST_NOATIME 0x400; STRICTATIME shows neither of the two; NODIRATIME adds
 * 0x800 to the default; NOSYMFOLLOW adds 0x2000 (a path through a symbolic
 * link in the mount is ELOOP). The atime choices are exclusive: the bits
 * 0x30 together, attribute bits that do not exist (0x400, 0x1000, bit
 * 31) and MOUNT_ATTR_IDMAP (0x100000, which needs a user namespace in
 * mount_setattr) are EINVAL. The attribute word is 32 bits wide.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_fsmount_attrs"

static const char *g_base;
static int g_n;

/* Make a mount with these attributes, attach it under base/aN, return the path. */
static const char *mount_with(u64 attrs, char *out) {
    i64 m = pm_tmpfs_fd("1m", attrs, FSMOUNT_CLOEXEC);
    if (m < 0) return 0;
    char name[8] = {'a', (char)('0' + g_n++), 0};
    pm_path(out, g_base, name);
    pm_mkdir(out);
    if (pm_attach(m, out) < 0) return 0;
    a_close(m);
    return out;
}

static void flags_of(const char *name, u64 attrs, i64 want) {
    char p[128];
    const char *path = mount_with(attrs, p);
    check(name, path != 0 && pm_mntflags(path) == want, path ? pm_mntflags(path) : -1);
}

static void body(const char *base) {
    g_base = base;
    flags_of("default_relatime", 0, ST_RELATIME);
    flags_of("rdonly", MOUNT_ATTR_RDONLY, ST_RELATIME | ST_RDONLY);
    flags_of("nosuid", MOUNT_ATTR_NOSUID, ST_RELATIME | ST_NOSUID);
    flags_of("nodev", MOUNT_ATTR_NODEV, ST_RELATIME | ST_NODEV);
    flags_of("noexec", MOUNT_ATTR_NOEXEC, ST_RELATIME | ST_NOEXEC);
    flags_of("noatime", MOUNT_ATTR_NOATIME, ST_NOATIME);
    flags_of("strictatime", MOUNT_ATTR_STRICTATIME, 0);
    flags_of("nodiratime", MOUNT_ATTR_NODIRATIME, ST_RELATIME | ST_NODIRATIME);
    flags_of("nosymfollow", MOUNT_ATTR_NOSYMFOLLOW, ST_RELATIME | ST_NOSYMFOLLOW);
    flags_of("several", MOUNT_ATTR_RDONLY | MOUNT_ATTR_NOSUID | MOUNT_ATTR_NODEV | MOUNT_ATTR_NOEXEC |
                            MOUNT_ATTR_NOATIME | MOUNT_ATTR_NODIRATIME,
             ST_RDONLY | ST_NOSUID | ST_NODEV | ST_NOEXEC | ST_NOATIME | ST_NODIRATIME);

    /* effects */
    char p[128];
    const char *ro = mount_with(MOUNT_ATTR_RDONLY, p);
    expect("rdonly_erofs", ro ? pm_can_create(ro) : -1, -EROFS);
    const char *rw = mount_with(0, p);
    expect("writable_without_rdonly", rw ? pm_can_create(rw) : -1, 0);

    const char *nx = mount_with(MOUNT_ATTR_NOEXEC, p);
    char px[160];
    expect("exec_file_noexec", nx ? a_make_file(pm_path(px, nx, "prog"), "not a program\n", 0755) : -1, 0);
    char *argv[] = {px, 0};
    char *envp[] = {0};
    expect("noexec_eacces", syscall3(SYS_execve, px, argv, envp), -EACCES);
    const char *ex = mount_with(0, p);
    char py[160];
    expect("exec_file_plain", ex ? a_make_file(pm_path(py, ex, "prog"), "not a program\n", 0755) : -1, 0);
    argv[0] = py;
    expect("plain_enoexec", syscall3(SYS_execve, py, argv, envp), -ENOEXEC);

    const char *nd = mount_with(MOUNT_ATTR_NODEV, p);
    char pd[160];
    expect("mknod_null", nd ? syscall4(SYS_mknodat, AT_FDCWD, pm_path(pd, nd, "null"), S_IFCHR | 0666, (1 << 8) | 3) : -1, 0);
    expect("nodev_open_eacces", a_open(pd, O_RDONLY, 0), -EACCES);
    const char *dv = mount_with(0, p);
    char pe[160];
    expect("mknod_null_plain", dv ? syscall4(SYS_mknodat, AT_FDCWD, pm_path(pe, dv, "null"), S_IFCHR | 0666, (1 << 8) | 3) : -1, 0);
    i64 nf = a_open(pe, O_RDONLY, 0);
    check("device_open_without_nodev", nf >= 0, nf);
    const char *sy = mount_with(MOUNT_ATTR_NOSYMFOLLOW, p);
    char ps[160], pt[160];
    expect("make_target", sy ? a_make_file(pm_path(pt, sy, "target"), "x", 0644) : -1, 0);
    expect("make_symlink", sy ? syscall3(SYS_symlinkat, "target", AT_FDCWD, pm_path(ps, sy, "link")) : -1, 0);
    expect("nosymfollow_eloop", a_open(ps, O_RDONLY, 0), -ELOOP);
    expect("direct_open_works", a_open(pt, O_RDONLY, 0) >= 0 ? 0 : -1, 0);

    /* bad attributes */
    i64 fc;
    const u64 bad[] = {MOUNT_ATTR_NOATIME | MOUNT_ATTR_STRICTATIME, 0x40, 0x50, 0x70, 0x400, 0x1000,
                       MOUNT_ATTR_IDMAP, 0x80000000UL};
    const char *names[] = {"noatime_and_strictatime", "atime_0x40", "atime_0x50", "atime_0x70", "bit_0x400",
                           "bit_0x1000", "idmap", "bit_31"};
    for (int i = 0; i < 8; i++) {
        fc = pm_fsopen("tmpfs", FSOPEN_CLOEXEC);
        pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0);
        expect(names[i], pm_fsmount(fc, 0, bad[i]), -EINVAL);
        a_close(fc);
    }
    /* a rejected call leaves the context usable */
    fc = pm_fsopen("tmpfs", FSOPEN_CLOEXEC);
    pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0);
    expect("bad_attr", pm_fsmount(fc, 0, 0x400), -EINVAL);
    i64 m = pm_fsmount(fc, FSMOUNT_CLOEXEC, MOUNT_ATTR_RDONLY);
    check("good_after_bad", m >= 0, m);
}

int test_main(void) { return pm_main(BASE, body); }
