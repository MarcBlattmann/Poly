/* lgetxattr on files that are neither regular files nor directories (all
 * capabilities dropped first): a FIFO, a UNIX socket file, the character
 * device /dev/null and a symbolic link to a FIFO. user.* names are ENODATA
 * there, even "user." and even when the mode gives no read permission;
 * trusted.* and security.* are ENODATA, "security." EINVAL; the ACL names
 * are ENODATA on the FIFO, socket and device but EOPNOTSUPP on the link;
 * other system.* names and unknown prefixes are EOPNOTSUPP, and unknown
 * prefixes need read permission first (a 0000 FIFO: EACCES). */
#include "a_sys.h"

#define SYS_socket 41
#define SYS_bind 49
#define SYS_capget 125
#define SYS_capset 126
#define SYS_mknod 133
#define SYS_lgetxattr 192
#define ENODATA 61
#define D "/tmp/a_lgetxattr_types"

static char buf[64];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 lget(const char *p, const char *name) { return syscall4(SYS_lgetxattr, p, name, buf, sizeof buf); }

static void names(const char *tag, const char *p, i64 acl) {
    char label[80];
    static const char *const enodata[] = { "user.a", "user.", "trusted.a", "security.a", 0 };
    static const char *const unsupported[] = { "system.x", "foo.a", 0 };
    for (int i = 0; enodata[i]; i++) {
        str_cpy(label, tag);
        str_cpy(label + str_len(label), "_");
        str_cpy(label + str_len(label), enodata[i]);
        expect(label, lget(p, enodata[i]), -ENODATA);
    }
    for (int i = 0; unsupported[i]; i++) {
        str_cpy(label, tag);
        str_cpy(label + str_len(label), "_");
        str_cpy(label + str_len(label), unsupported[i]);
        expect(label, lget(p, unsupported[i]), -EOPNOTSUPP);
    }
    str_cpy(label, tag);
    str_cpy(label + str_len(label), "_acl_access");
    expect(label, lget(p, "system.posix_acl_access"), acl);
    str_cpy(label, tag);
    str_cpy(label + str_len(label), "_acl_default");
    expect(label, lget(p, "system.posix_acl_default"), acl);
    str_cpy(label, tag);
    str_cpy(label + str_len(label), "_security_dot");
    expect(label, lget(p, "security."), -EINVAL);
}

int test_main(void) {
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped != 0) return finish();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_fifo", syscall3(SYS_mknod, D "/p", S_IFIFO | 0644, 0), 0);
    expect("setup_link", syscall2(SYS_symlink, "p", D "/l"), 0);
    i64 s = syscall3(SYS_socket, 1, 1, 0); /* AF_UNIX, SOCK_STREAM */
    check("setup_socket", s >= 0, s);
    struct {
        u16 family;
        char path[64];
    } sa = { 1, D "/sock" };
    expect("setup_bind", syscall3(SYS_bind, s, &sa, sizeof sa), 0);
    names("fifo", D "/p", -ENODATA);
    names("socket", D "/sock", -ENODATA);
    names("devnull", "/dev/null", -ENODATA);
    names("link", D "/l", -EOPNOTSUPP);
    expect("setup_fifo_none", syscall3(SYS_fchmodat, AT_FDCWD, D "/p", 0), 0);
    expect("fifo_none_user", lget(D "/p", "user.a"), -ENODATA);
    expect("fifo_none_user_dot", lget(D "/p", "user."), -ENODATA);
    expect("fifo_none_security", lget(D "/p", "security.a"), -ENODATA);
    expect("fifo_none_unknown_eacces", lget(D "/p", "foo.a"), -EACCES);
    expect("link_to_none_unknown", lget(D "/l", "foo.a"), -EOPNOTSUPP);
    a_close(s);
    expect("cleanup_sock", a_unlink(D "/sock"), 0);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_p", a_unlink(D "/p"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
