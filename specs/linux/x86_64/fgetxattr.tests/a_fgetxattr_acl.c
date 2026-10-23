/* POSIX ACLs read through a descriptor (all capabilities dropped): an
 * access ACL with a named-user entry reads back in the xattr layout of
 * linux/posix_acl_xattr.h (u32 version 2, then 8-byte entries u16 tag,
 * u16 perm, u32 id; ids of the non-named entries 0xffffffff). Size 0 gives
 * its length, a short buffer ERANGE. A chmod rewrites the owner, mask and
 * other entries, and the ACL stays readable without read permission (mode
 * 0000, opened before). A directory's default ACL reads back through an
 * O_DIRECTORY descriptor; its access ACL stays ENODATA. Tags from
 * linux/posix_acl.h: USER_OBJ 1, USER 2, GROUP_OBJ 4, MASK 0x10, OTHER
 * 0x20. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_fsetxattr 190
#define SYS_fgetxattr 193
#define ENODATA 61
#define D "/tmp/a_fgetxattr_acl"
#define ACL_ACCESS "system.posix_acl_access"
#define ACL_DEFAULT "system.posix_acl_default"

static u8 acl[64], got[64];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static void put16(u8 *p, u32 v) { p[0] = v; p[1] = v >> 8; }
static void put32(u8 *p, u32 v) { p[0] = v; p[1] = v >> 8; p[2] = v >> 16; p[3] = v >> 24; }

static int entry(u8 *base, int i, u32 tag, u32 perm, u32 id) {
    u8 *e = base + 4 + 8 * i;
    put16(e, tag);
    put16(e + 2, perm);
    put32(e + 4, id);
    return 4 + 8 * (i + 1);
}

int test_main(void) {
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped != 0) return finish();
    u32 uid = (u32)syscall0(SYS_getuid);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    check("setup_open", fd >= 0, fd);
    expect("none_yet", syscall4(SYS_fgetxattr, fd, ACL_ACCESS, got, sizeof got), -ENODATA);
    put32(acl, 2);
    entry(acl, 0, 1, 6, 0xffffffff);
    entry(acl, 1, 2, 4, uid);
    entry(acl, 2, 4, 4, 0xffffffff);
    entry(acl, 3, 0x10, 4, 0xffffffff);
    int len = entry(acl, 4, 0x20, 4, 0xffffffff);
    expect("set_access", syscall5(SYS_fsetxattr, fd, ACL_ACCESS, acl, len, 0), 0);
    expect("size0", syscall4(SYS_fgetxattr, fd, ACL_ACCESS, 0, 0), 44);
    expect("short_erange", syscall4(SYS_fgetxattr, fd, ACL_ACCESS, got, 43), -ERANGE);
    expect("read", syscall4(SYS_fgetxattr, fd, ACL_ACCESS, got, sizeof got), 44);
    check("read_bytes", mem_eq(got, acl, 44), got[4]);
    expect("fchmod_0000", syscall2(SYS_fchmod, fd, 0), 0);
    expect("read_mode0", syscall4(SYS_fgetxattr, fd, ACL_ACCESS, got, sizeof got), 44);
    entry(acl, 0, 1, 0, 0xffffffff);
    entry(acl, 3, 0x10, 0, 0xffffffff);
    entry(acl, 4, 0x20, 0, 0xffffffff);
    check("mode0_rewrote_entries", mem_eq(got, acl, 44), got[6]);
    expect("mode0_user_eacces", syscall4(SYS_fgetxattr, fd, "user.a", got, sizeof got), -EACCES);
    expect("default_on_file", syscall4(SYS_fgetxattr, fd, ACL_DEFAULT, got, sizeof got), -ENODATA);
    a_close(fd);
    expect("setup_dir", a_mkdir(D "/d", 0755), 0);
    fd = a_open(D "/d", O_RDONLY | O_DIRECTORY, 0);
    check("setup_open_dir", fd >= 0, fd);
    put32(acl, 2);
    entry(acl, 0, 1, 7, 0xffffffff);
    entry(acl, 1, 4, 5, 0xffffffff);
    len = entry(acl, 2, 0x20, 1, 0xffffffff);
    expect("set_default", syscall5(SYS_fsetxattr, fd, ACL_DEFAULT, acl, len, 0), 0);
    expect("read_default", syscall4(SYS_fgetxattr, fd, ACL_DEFAULT, got, sizeof got), 28);
    check("default_bytes", mem_eq(got, acl, 28), got[4]);
    expect("dir_access_none", syscall4(SYS_fgetxattr, fd, ACL_ACCESS, got, sizeof got), -ENODATA);
    a_close(fd);
    expect("cleanup_d", a_rmdir(D "/d"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
