/* fsetxattr with fd AT_FDCWD (-100, also passed as 0xffffff9c since only
 * the low 32 bits count) acts on the current working directory: the
 * attribute lands on the directory, XATTR_CREATE then gives EEXIST, a
 * read-only (0555) working directory is EACCES, trusted.* without
 * CAP_SYS_ADMIN is EPERM, and a removed working directory still takes
 * attributes. (Assumes no CAP_DAC_OVERRIDE: capabilities are dropped
 * first.) */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_fsetxattr 190
#define SYS_getxattr 191
#define SYS_fgetxattr 193
#define D "/tmp/a_fsetxattr_atfdcwd"

static char buf[64];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 fset(i64 fd, const char *name, const char *v, i64 flags) {
    return syscall5(SYS_fsetxattr, fd, name, v, str_len(v), flags);
}

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_gone", a_mkdir(D "/gone", 0755), 0);
    expect("chdir_sub", syscall1(SYS_chdir, D "/sub"), 0);
    expect("cwd_set", fset(AT_FDCWD, "user.c", "cwd", 0), 0);
    expect("cwd_get", syscall4(SYS_getxattr, D "/sub", "user.c", buf, sizeof buf), 3);
    check("cwd_bytes", mem_eq(buf, "cwd", 3), 0);
    expect("parent_untouched", syscall4(SYS_getxattr, D, "user.c", buf, sizeof buf), -61 /* ENODATA */);
    expect("cwd_create_eexist", fset(AT_FDCWD, "user.c", "x", 1), -EEXIST);
    expect("cwd_u32_form", fset(0xffffff9cLL, "user.u", "u32", 0), 0);
    expect("cwd_u32_get", syscall4(SYS_getxattr, D "/sub", "user.u", buf, sizeof buf), 3);
    expect("cwd_trusted_eperm", fset(AT_FDCWD, "trusted.c", "x", 0), -EPERM);
    expect("chmod_555", syscall3(SYS_fchmodat, AT_FDCWD, D "/sub", 0555), 0);
    expect("cwd_ro_eacces", fset(AT_FDCWD, "user.r", "x", 0), -EACCES);
    expect("chmod_755", syscall3(SYS_fchmodat, AT_FDCWD, D "/sub", 0755), 0);
    expect("chdir_gone", syscall1(SYS_chdir, D "/gone"), 0);
    i64 keep = a_open(".", O_RDONLY | O_DIRECTORY, 0);
    check("keep_open", keep >= 0, keep);
    expect("rmdir_cwd", a_rmdir(D "/gone"), 0);
    expect("removed_cwd_set", fset(AT_FDCWD, "user.g", "gg", 0), 0);
    expect("removed_cwd_get", syscall4(SYS_fgetxattr, keep, "user.g", buf, sizeof buf), 2);
    a_close(keep);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    a_rmdir(D "/sub");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
