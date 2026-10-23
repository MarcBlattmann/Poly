/* POSIX ACLs show up in the list: a stored access ACL as
 * system.posix_acl_access, a directory's default ACL as
 * system.posix_acl_default (24 and 25 bytes with the NUL), and a new
 * subdirectory inherits the default ACL. An ACL with only owner, group and
 * other entries is not stored, so it is not listed; a removed ACL is gone.
 * ACL layout from linux/posix_acl_xattr.h, tags from linux/posix_acl.h. */
#include "a_sys.h"

#define SYS_fsetxattr 190
#define SYS_flistxattr 196
#define SYS_fremovexattr 199
#define D "/tmp/a_flistxattr_acl"
#define ACCESS "system.posix_acl_access"
#define DEFAULT "system.posix_acl_default"

struct ent {
    unsigned short tag, perm;
    unsigned id;
};

static char buf[256];
static i64 len;

static i64 flist(i64 fd) { return len = syscall3(SYS_flistxattr, fd, buf, sizeof buf); }

static int has(const char *name) {
    int n = 0;
    for (i64 i = 0; i < len; i += str_len(buf + i) + 1)
        if (str_len(buf + i) == str_len(name) && mem_eq(buf + i, name, str_len(name))) n++;
    return n;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_dir", a_mkdir(D "/d", 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_min_file", a_make_file(D "/m", "", 0644), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    i64 mf = a_open(D "/m", O_RDONLY, 0);
    i64 dd = a_open(D "/d", O_RDONLY | O_DIRECTORY, 0);
    check("setup_open", fd >= 0 && mf >= 0 && dd >= 0, fd);
    struct {
        unsigned version;
        struct ent e[5];
    } full = { 2, { { 1, 6, ~0u }, { 2, 4, (unsigned)syscall0(SYS_geteuid) }, { 4, 4, ~0u }, { 0x10, 4, ~0u }, { 0x20, 4, ~0u } } };
    struct {
        unsigned version;
        struct ent e[3];
    } min = { 2, { { 1, 6, ~0u }, { 4, 4, ~0u }, { 0x20, 4, ~0u } } };
    struct {
        unsigned version;
        struct ent e[5];
    } dir = { 2, { { 1, 7, ~0u }, { 2, 5, (unsigned)syscall0(SYS_geteuid) }, { 4, 5, ~0u }, { 0x10, 5, ~0u }, { 0x20, 5, ~0u } } };
    expect("setup_set_access", syscall5(SYS_fsetxattr, fd, ACCESS, &full, sizeof full, 0), 0);
    expect("access", flist(fd), 24);
    check("access_bytes", mem_eq(buf, ACCESS "\0", 24), 0);
    expect("setup_set_user", syscall5(SYS_fsetxattr, fd, "user.u", "", 0, 0), 0);
    expect("access_and_user", flist(fd), 31);
    expect("access_and_user_acl", has(ACCESS), 1);
    expect("access_and_user_user", has("user.u"), 1);
    expect("setup_remove_access", syscall2(SYS_fremovexattr, fd, ACCESS), 0);
    expect("access_removed", flist(fd), 7);
    expect("access_removed_gone", has(ACCESS), 0);
    expect("setup_set_min", syscall5(SYS_fsetxattr, mf, ACCESS, &min, sizeof min, 0), 0);
    expect("min_not_listed", flist(mf), 0);
    expect("setup_set_default", syscall5(SYS_fsetxattr, dd, DEFAULT, &min, sizeof min, 0), 0);
    expect("default", flist(dd), 25);
    check("default_bytes", mem_eq(buf, DEFAULT "\0", 25), 0);
    expect("setup_subdir", a_mkdir(D "/d/s", 0755), 0);
    i64 sd = a_open(D "/d/s", O_RDONLY | O_DIRECTORY, 0);
    check("setup_open_subdir", sd >= 0, sd);
    expect("subdir_inherits", flist(sd), 25);
    expect("subdir_default", has(DEFAULT), 1);
    expect("setup_set_dir_access", syscall5(SYS_fsetxattr, dd, ACCESS, &dir, sizeof dir, 0), 0);
    expect("dir_both", flist(dd), 49);
    expect("dir_both_access", has(ACCESS), 1);
    expect("dir_both_default", has(DEFAULT), 1);
    a_close(sd);
    a_close(dd);
    a_close(mf);
    a_close(fd);
    expect("cleanup_s", a_rmdir(D "/d/s"), 0);
    expect("cleanup_d", a_rmdir(D "/d"), 0);
    expect("cleanup_m", a_unlink(D "/m"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
