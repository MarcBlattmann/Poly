/* Order of checks when several problems exist (all capabilities dropped):
 * name (EFAULT, empty or 256+ bytes ERANGE) before the descriptor
 * (EBADF); then on a file without read permission EACCES for user.* and
 * unknown prefixes, before EOPNOTSUPP, EINVAL ("user."), ENODATA, ERANGE
 * and EFAULT from the buffer; trusted.* is ENODATA before anything else
 * on the file; on a readable file an unknown prefix is EOPNOTSUPP whatever
 * the size or buffer, a missing name ENODATA before buffer checks, and
 * ERANGE (size too small) before EFAULT. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_fsetxattr 190
#define SYS_fgetxattr 193
#define ENODATA 61
#define EBADF 9
#define D "/tmp/a_fgetxattr_order"
#define BAD ((void *)8)

static char longname[300];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 fg(i64 fd, const void *name, void *b, u64 size) { return syscall4(SYS_fgetxattr, fd, name, b, size); }

int test_main(void) {
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped != 0) return finish();
    for (int i = 0; i < 256; i++) longname[i] = i < 5 ? "user."[i] : 'n';
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    i64 fd = a_open(D "/f", O_RDWR, 0);
    check("setup_open", fd >= 0, fd);
    expect("setup_set", syscall5(SYS_fsetxattr, fd, "user.a", "abcd", 4, 0), 0);
    longname[255] = 0;
    expect("long_255_ok_missing", fg(fd, longname, 0, 0), -ENODATA);
    longname[255] = 'n';
    expect("long_256_erange", fg(fd, longname, 0, 0), -ERANGE);
    expect("long_256_badfd", fg(-1, longname, 0, 0), -ERANGE);
    expect("name_fault_badfd", fg(-1, BAD, 0, 0), -EFAULT);
    expect("readable_unknown_null_buf", fg(fd, "foo.a", 0, 1), -EOPNOTSUPP);
    expect("readable_user_dot_null_buf", fg(fd, "user.", 0, 1), -EINVAL);
    expect("readable_missing_bad_buf", fg(fd, "user.b", BAD, 64), -ENODATA);
    expect("readable_erange_bad_buf", fg(fd, "user.a", BAD, 3), -ERANGE);
    expect("readable_efault_last", fg(fd, "user.a", BAD, 4), -EFAULT);
    expect("fchmod_0200", syscall2(SYS_fchmod, fd, 0200), 0);
    expect("noread_unknown", fg(fd, "foo.a", 0, 0), -EACCES);
    expect("noread_user_dot", fg(fd, "user.", 0, 0), -EACCES);
    expect("noread_missing", fg(fd, "user.b", 0, 0), -EACCES);
    expect("noread_erange", fg(fd, "user.a", 0, 1), -EACCES);
    expect("noread_efault", fg(fd, "user.a", BAD, 64), -EACCES);
    expect("noread_empty_name", fg(fd, "", 0, 0), -ERANGE);
    expect("noread_trusted", fg(fd, "trusted.a", BAD, 64), -ENODATA);
    expect("noread_trusted_dot", fg(fd, "trusted.", 0, 0), -ENODATA);
    expect("noread_system_x", fg(fd, "system.x", 0, 0), -EOPNOTSUPP);
    expect("noread_security_dot", fg(fd, "security.", 0, 0), -EINVAL);
    a_close(fd);
    expect("closed_unknown", fg(fd, "foo.a", 0, 0), -EBADF);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
