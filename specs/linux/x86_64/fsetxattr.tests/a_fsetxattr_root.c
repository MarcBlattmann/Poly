/* fsetxattr as root: CAP_SYS_ADMIN allows trusted.* through any
 * descriptor of a regular file or directory (O_RDONLY included, AT_FDCWD
 * too), with XATTR_CREATE / XATTR_REPLACE working as for user.*, and
 * "trusted." alone is EINVAL. CAP_DAC_OVERRIDE lets root set user.* on a
 * 0444 file through an O_RDONLY descriptor. user.* on a pipe stays EPERM,
 * and an O_PATH descriptor stays EBADF. After all capabilities are
 * dropped, trusted.* is EPERM and the 0444 file EACCES.
 * poly-diff: needs=root (trusted.* needs CAP_SYS_ADMIN)
 */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_fsetxattr 190
#define SYS_getxattr 191
#define SYS_fgetxattr 193
#define ENODATA 61
#define D "/tmp/a_fsetxattr_root"

static char buf[64];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 fset(i64 fd, const char *n, const char *v, i64 flags) {
    return syscall5(SYS_fsetxattr, fd, n, v, str_len(v), flags);
}

int test_main(void) {
    i64 uid = syscall0(SYS_geteuid);
    expect("is_root", uid, 0);
    if (uid != 0) return finish();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_ro", a_make_file(D "/ro", "", 0444), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    check("setup_open_f", fd >= 0, fd);
    expect("trusted_set", fset(fd, "trusted.t", "root", 0), 0);
    expect("trusted_get", syscall4(SYS_getxattr, D "/f", "trusted.t", buf, sizeof buf), 4);
    check("trusted_bytes", mem_eq(buf, "root", 4), 0);
    expect("trusted_create_eexist", fset(fd, "trusted.t", "x", 1), -EEXIST);
    expect("trusted_replace_missing", fset(fd, "trusted.m", "x", 2), -ENODATA);
    expect("trusted_replace", fset(fd, "trusted.t", "rt", 2), 0);
    expect("trusted_replaced", syscall4(SYS_fgetxattr, fd, "trusted.t", buf, sizeof buf), 2);
    expect("trusted_dot_einval", fset(fd, "trusted.", "x", 0), -EINVAL);
    i64 ro = a_open(D "/ro", O_RDONLY, 0);
    check("setup_open_ro", ro >= 0, ro);
    expect("ro_user_ok", fset(ro, "user.u", "ro", 0), 0);
    expect("ro_user_get", syscall4(SYS_fgetxattr, ro, "user.u", buf, sizeof buf), 2);
    int p[2];
    expect("setup_pipe", syscall2(SYS_pipe2, p, 0), 0);
    expect("pipe_user_eperm", fset(p[0], "user.a", "x", 0), -EPERM);
    a_close(p[0]);
    a_close(p[1]);
    i64 op = a_open(D "/f", O_PATH, 0);
    check("setup_opath", op >= 0, op);
    expect("opath_trusted_ebadf", fset(op, "trusted.o", "x", 0), -EBADF);
    a_close(op);
    expect("chdir_sub", syscall1(SYS_chdir, D "/sub"), 0);
    expect("cwd_trusted", fset(AT_FDCWD, "trusted.c", "cwd", 0), 0);
    expect("cwd_trusted_get", syscall4(SYS_getxattr, D "/sub", "trusted.c", buf, sizeof buf), 3);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped == 0) {
        expect("dropped_trusted_eperm", fset(fd, "trusted.t", "x", 0), -EPERM);
        expect("dropped_ro_eacces", fset(ro, "user.v", "x", 0), -EACCES);
    }
    a_close(fd);
    a_close(ro);
    a_unlink(D "/f");
    a_unlink(D "/ro");
    a_rmdir(D "/sub");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
