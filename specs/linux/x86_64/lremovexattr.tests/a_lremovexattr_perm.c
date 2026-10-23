/* lremovexattr permission checks, with all capabilities dropped first:
 * user.* removal needs write permission on the file by its mode bits:
 * a 0444 file or a 0555 directory gives EACCES (whether the name exists
 * or not) and the attribute stays; a write-only 0200 file works. A link
 * pointing at a 0444 file is still the link (EPERM, not EACCES). The
 * owner of a sticky directory may remove its attributes. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_getxattr 191
#define SYS_lremovexattr 198
#define ENODATA 61
#define D "/tmp/a_lremovexattr_perm"

static char buf[8];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 lrm(const char *p, const char *name) { return syscall2(SYS_lremovexattr, p, name); }
static i64 set(const char *p, const char *name) { return syscall5(SYS_setxattr, p, name, "v", 1, 0); }
static i64 get(const char *p, const char *name) { return syscall4(SYS_getxattr, p, name, buf, sizeof buf); }
static i64 chmod(const char *p, i64 mode) { return syscall3(SYS_fchmodat, AT_FDCWD, p, mode); }

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_ro", a_make_file(D "/ro", "", 0644), 0);
    expect("set_ro", set(D "/ro", "user.a"), 0);
    expect("chmod_ro", chmod(D "/ro", 0444), 0);
    expect("ro_eacces", lrm(D "/ro", "user.a"), -EACCES);
    expect("ro_kept", get(D "/ro", "user.a"), 1);
    expect("ro_missing_eacces", lrm(D "/ro", "user.missing"), -EACCES);
    expect("setup_link_to_ro", syscall3(SYS_symlinkat, "ro", AT_FDCWD, D "/l"), 0);
    expect("link_to_ro_eperm", lrm(D "/l", "user.a"), -EPERM);
    expect("link_to_ro_unknown_eopnotsupp", lrm(D "/l", "foo.bar"), -EOPNOTSUPP);
    expect("setup_wo", a_make_file(D "/wo", "", 0644), 0);
    expect("set_wo", set(D "/wo", "user.a"), 0);
    expect("chmod_wo", chmod(D "/wo", 0200), 0);
    expect("wo_ok", lrm(D "/wo", "user.a"), 0);
    expect("wo_again_enodata", lrm(D "/wo", "user.a"), -ENODATA);
    expect("setup_rodir", a_mkdir(D "/rodir", 0755), 0);
    expect("set_rodir", set(D "/rodir", "user.a"), 0);
    expect("chmod_rodir", chmod(D "/rodir", 0555), 0);
    expect("rodir_eacces", lrm(D "/rodir", "user.a"), -EACCES);
    expect("rodir_unknown_eacces", lrm(D "/rodir", "foo.bar"), -EACCES);
    expect("chmod_rodir_back", chmod(D "/rodir", 0755), 0);
    expect("rodir_ok", lrm(D "/rodir", "user.a"), 0);
    expect("setup_sticky", a_mkdir(D "/sticky", 0755), 0);
    expect("chmod_sticky", chmod(D "/sticky", 01777), 0);
    expect("set_sticky", set(D "/sticky", "user.a"), 0);
    expect("sticky_owner_ok", lrm(D "/sticky", "user.a"), 0);
    expect("sticky_again_enodata", lrm(D "/sticky", "user.a"), -ENODATA);
    a_rmdir(D "/sticky");
    a_rmdir(D "/rodir");
    a_unlink(D "/l");
    a_unlink(D "/wo");
    expect("cleanup_ro", a_unlink(D "/ro"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
