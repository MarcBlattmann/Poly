/* system.posix_acl_access and system.posix_acl_default read back a set
 * ACL in the same xattr format it was set in (version 2 header, then
 * 8-byte entries: tag, permissions, id); size 0 gives its length and a
 * short buffer is ERANGE. An ACL that only repeats the mode (owner, group,
 * other entries) is not kept: it sets the mode and reads as ENODATA. A
 * chmod rewrites the stored ACL's owner and mask entries. A directory's
 * default ACL is separate from its access ACL. ACL names need no read
 * permission on the file (all capabilities dropped first). */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_getxattr 191
#define ENODATA 61
#define D "/tmp/a_getxattr_acl"
#define ACCESS "system.posix_acl_access"
#define DEFAULT "system.posix_acl_default"
#define NOID 0xffffffffu

static unsigned char buf[128];
static unsigned char acl[64];
static int len;

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static void start(void) {
    acl[0] = 2, acl[1] = acl[2] = acl[3] = 0;
    len = 4;
}

/* linux/posix_acl_xattr.h: u16 tag, u16 perm, u32 id, little endian. */
static void entry(int tag, int perm, u32 id) {
    unsigned char *e = acl + len;
    e[0] = (unsigned char)tag, e[1] = (unsigned char)(tag >> 8), e[2] = (unsigned char)perm, e[3] = 0;
    e[4] = (unsigned char)id, e[5] = (unsigned char)(id >> 8), e[6] = (unsigned char)(id >> 16);
    e[7] = (unsigned char)(id >> 24);
    len += 8;
}

static i64 get(const char *p, const char *name) { return syscall4(SYS_getxattr, p, name, buf, sizeof buf); }

int test_main(void) {
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped != 0) return finish();
    u32 uid = (u32)syscall0(SYS_geteuid);
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    /* USER_OBJ rw-, USER(uid) r--, GROUP_OBJ r--, MASK r--, OTHER --- */
    start();
    entry(0x01, 6, NOID);
    entry(0x02, 4, uid);
    entry(0x04, 4, NOID);
    entry(0x10, 4, NOID);
    entry(0x20, 0, NOID);
    expect("set_acl", syscall5(SYS_setxattr, D "/f", ACCESS, acl, len, 0), 0);
    expect("get_acl", get(D "/f", ACCESS), 44);
    check("acl_same_bytes", mem_eq(buf, acl, 44), 0);
    expect("acl_size0", syscall4(SYS_getxattr, D "/f", ACCESS, 0, 0), 44);
    expect("acl_short_erange", syscall4(SYS_getxattr, D "/f", ACCESS, buf, 43), -ERANGE);
    expect("acl_sets_mode", a_stat_path(D "/f", &st), 0);
    expect("acl_mode", st.st_mode & 07777, 0640);
    expect("chmod_0", syscall3(SYS_fchmodat, AT_FDCWD, D "/f", 0), 0);
    expect("get_acl_mode_0", get(D "/f", ACCESS), 44);
    acl[4 + 2] = 0;      /* USER_OBJ */
    acl[4 + 3 * 8 + 2] = 0; /* MASK */
    check("chmod_rewrote_acl", mem_eq(buf, acl, 44), 0);
    expect("user_name_still_eacces", get(D "/f", "user.a"), -EACCES);
    expect("setup_min_file", a_make_file(D "/m", "", 0644), 0);
    start();
    entry(0x01, 7, NOID);
    entry(0x04, 5, NOID);
    entry(0x20, 0, NOID);
    expect("set_minimal", syscall5(SYS_setxattr, D "/m", ACCESS, acl, len, 0), 0);
    expect("minimal_enodata", get(D "/m", ACCESS), -ENODATA);
    expect("minimal_stat", a_stat_path(D "/m", &st), 0);
    expect("minimal_mode", st.st_mode & 07777, 0750);
    expect("setup_dir", a_mkdir(D "/d", 0755), 0);
    start();
    entry(0x01, 7, NOID);
    entry(0x04, 5, NOID);
    entry(0x20, 1, NOID);
    expect("set_default", syscall5(SYS_setxattr, D "/d", DEFAULT, acl, len, 0), 0);
    expect("get_default", get(D "/d", DEFAULT), 28);
    check("default_same_bytes", mem_eq(buf, acl, 28), 0);
    expect("dir_access_enodata", get(D "/d", ACCESS), -ENODATA);
    expect("cleanup_d", a_rmdir(D "/d"), 0);
    expect("cleanup_m", a_unlink(D "/m"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
