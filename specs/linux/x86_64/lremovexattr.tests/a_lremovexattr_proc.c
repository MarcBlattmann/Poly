/* lremovexattr on /proc/self links, with all capabilities dropped first:
 * /proc/self/fd/N, /proc/self/exe and /proc/self/cwd are symbolic links,
 * so the link itself is the object and the open file keeps its attribute.
 * user.*, trusted.*, security.* are EPERM, ACL names and system.x
 * EOPNOTSUPP. For an unknown prefix the link's own mode counts: the link
 * of a read-only descriptor gives EACCES, that of a read-write descriptor
 * and /proc/self/exe or cwd EOPNOTSUPP. A trailing "/." after the cwd link
 * reaches the directory. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_chdir 80
#define SYS_setxattr 188
#define SYS_getxattr 191
#define SYS_lremovexattr 198
#define ENODATA 61
#define D "/tmp/a_lremovexattr_proc"

static char ro[48], rw[48], buf[8];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 lrm(const char *p, const char *name) { return syscall2(SYS_lremovexattr, p, name); }
static i64 get(const char *p, const char *name) { return syscall4(SYS_getxattr, p, name, buf, sizeof buf); }

static char *fd_link(char *out, i64 fd) {
    str_cpy(out, "/proc/self/fd/");
    a_utoa(out + str_len(out), fd);
    return out;
}

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("set_f", syscall5(SYS_setxattr, D "/f", "user.a", "v", 1, 0), 0);
    i64 fdr = a_open(D "/f", O_RDONLY, 0);
    i64 fdw = a_open(D "/f", O_RDWR, 0);
    check("open_ro", fdr >= 0, fdr);
    check("open_rw", fdw >= 0, fdw);
    fd_link(ro, fdr);
    fd_link(rw, fdw);
    expect("ro_user_eperm", lrm(ro, "user.a"), -EPERM);
    expect("ro_trusted_eperm", lrm(ro, "trusted.a"), -EPERM);
    expect("ro_security_eperm", lrm(ro, "security.a"), -EPERM);
    expect("ro_acl_eopnotsupp", lrm(ro, "system.posix_acl_access"), -EOPNOTSUPP);
    expect("ro_system_x_eopnotsupp", lrm(ro, "system.x"), -EOPNOTSUPP);
    expect("ro_unknown_eacces", lrm(ro, "foo.bar"), -EACCES);
    expect("ro_empty_erange", lrm(ro, ""), -ERANGE);
    expect("rw_user_eperm", lrm(rw, "user.a"), -EPERM);
    expect("rw_unknown_eopnotsupp", lrm(rw, "foo.bar"), -EOPNOTSUPP);
    expect("rw_default_acl_eopnotsupp", lrm(rw, "system.posix_acl_default"), -EOPNOTSUPP);
    expect("file_kept", get(D "/f", "user.a"), 1);
    expect("exe_user_eperm", lrm("/proc/self/exe", "user.a"), -EPERM);
    expect("exe_unknown_eopnotsupp", lrm("/proc/self/exe", "foo.bar"), -EOPNOTSUPP);
    expect("chdir", syscall1(SYS_chdir, D), 0);
    expect("set_dir", syscall5(SYS_setxattr, D, "user.d", "v", 1, 0), 0);
    expect("cwd_user_eperm", lrm("/proc/self/cwd", "user.d"), -EPERM);
    expect("cwd_unknown_eopnotsupp", lrm("/proc/self/cwd", "foo.bar"), -EOPNOTSUPP);
    expect("dir_kept", get(D, "user.d"), 1);
    expect("cwd_dot_follows", lrm("/proc/self/cwd/.", "user.d"), 0);
    expect("dir_gone", get(D, "user.d"), -ENODATA);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("unlink_open_file", a_unlink(D "/f"), 0);
    expect("unlinked_ro_user_eperm", lrm(ro, "user.a"), -EPERM);
    a_close(fdr);
    a_close(fdw);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
