/* With all capabilities dropped, user.* and unknown prefixes need read
 * permission by the file's mode at the time of the call, not by how the
 * descriptor was opened: after fchmod 0200 an O_RDWR descriptor is EACCES
 * (also for "user." and for a name that does not exist), after fchmod 0400
 * it reads again, and the owner of a 0044 file is EACCES (owner bits
 * decide). trusted.*, security.* and the ACL names need no read
 * permission (ENODATA when absent); system.x is EOPNOTSUPP. The parent
 * directory's mode does not matter once the file is open. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_fgetxattr 193
#define ENODATA 61
#define D "/tmp/a_fgetxattr_perm"

static char buf[64];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 fget(i64 fd, const char *name) { return syscall4(SYS_fgetxattr, fd, name, buf, sizeof buf); }

int test_main(void) {
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped != 0) return finish();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/sub/f", "", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, D "/sub/f", "user.a", "abc", 3, 0), 0);
    i64 fd = a_open(D "/sub/f", O_RDWR, 0);
    check("setup_open", fd >= 0, fd);
    expect("readable", fget(fd, "user.a"), 3);
    expect("fchmod_0200", syscall2(SYS_fchmod, fd, 0200), 0);
    expect("w_only_user", fget(fd, "user.a"), -EACCES);
    expect("w_only_user_missing", fget(fd, "user.zz"), -EACCES);
    expect("w_only_user_dot", fget(fd, "user."), -EACCES);
    expect("w_only_unknown_prefix", fget(fd, "foo.a"), -EACCES);
    expect("w_only_size0", syscall4(SYS_fgetxattr, fd, "user.a", 0, 0), -EACCES);
    expect("w_only_trusted", fget(fd, "trusted.a"), -ENODATA);
    expect("w_only_security", fget(fd, "security.a"), -ENODATA);
    expect("w_only_acl_access", fget(fd, "system.posix_acl_access"), -ENODATA);
    expect("w_only_acl_default", fget(fd, "system.posix_acl_default"), -ENODATA);
    expect("w_only_system_x", fget(fd, "system.x"), -EOPNOTSUPP);
    expect("fchmod_0400", syscall2(SYS_fchmod, fd, 0400), 0);
    expect("r_only_reads", fget(fd, "user.a"), 3);
    expect("fchmod_0044", syscall2(SYS_fchmod, fd, 0044), 0);
    expect("owner_bits_decide", fget(fd, "user.a"), -EACCES);
    expect("fchmod_0000", syscall2(SYS_fchmod, fd, 0), 0);
    expect("mode0_user", fget(fd, "user.a"), -EACCES);
    expect("mode0_trusted", fget(fd, "trusted.a"), -ENODATA);
    expect("fchmod_0644", syscall2(SYS_fchmod, fd, 0644), 0);
    expect("parent_mode_0", syscall2(SYS_chmod, D "/sub", 0), 0);
    expect("parent_mode_ignored", fget(fd, "user.a"), 3);
    expect("parent_mode_back", syscall2(SYS_chmod, D "/sub", 0755), 0);
    a_close(fd);
    expect("setup_wo", a_make_file(D "/wo", "", 0200), 0);
    fd = a_open(D "/wo", O_WRONLY, 0);
    check("setup_open_wo", fd >= 0, fd);
    expect("opened_wronly_0200", fget(fd, "user.a"), -EACCES);
    a_close(fd);
    expect("cleanup_wo", a_unlink(D "/wo"), 0);
    expect("cleanup_f", a_unlink(D "/sub/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
