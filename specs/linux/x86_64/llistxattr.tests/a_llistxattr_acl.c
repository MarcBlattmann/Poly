/* llistxattr lists stored POSIX ACLs of files and directories like
 * listxattr: "system.posix_acl_access" for an access ACL and
 * "system.posix_acl_default" for a directory's default ACL; a new
 * subdirectory inherits both. A symbolic link to such a directory lists
 * as empty (links have no ACLs), and with a trailing slash lists the
 * directory's names. ACL xattr format from linux/posix_acl_xattr.h
 * (version 2, 8-byte entries). */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_llistxattr 195
#define D "/tmp/a_llistxattr_acl"
#define ACCESS "system.posix_acl_access"
#define DEFAULT "system.posix_acl_default"

static char buf[512];
static i64 len;
static unsigned char acl[4 + 8 * 5];

static i64 llist(const char *p) { return len = syscall3(SYS_llistxattr, p, buf, sizeof buf); }

static int has(const char *name) {
    int n = 0;
    for (i64 i = 0; i < len; i += str_len(buf + i) + 1)
        if (str_len(buf + i) == str_len(name) && mem_eq(buf + i, name, str_len(name))) n++;
    return n;
}

static void entry(int i, u32 tag, u32 perm, u32 id) {
    unsigned char *e = acl + 4 + 8 * i;
    e[0] = tag;
    e[1] = tag >> 8;
    e[2] = perm;
    e[3] = 0;
    e[4] = id;
    e[5] = id >> 8;
    e[6] = id >> 16;
    e[7] = id >> 24;
}

int test_main(void) {
    acl[0] = 2;
    /* ACL_USER_OBJ 1, ACL_USER 2, ACL_GROUP_OBJ 4, ACL_MASK 0x10, ACL_OTHER 0x20 */
    entry(0, 1, 7, 0xffffffff);
    entry(1, 2, 5, 12345);
    entry(2, 4, 5, 0xffffffff);
    entry(3, 0x10, 5, 0xffffffff);
    entry(4, 0x20, 5, 0xffffffff);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_dir", a_mkdir(D "/d", 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_link", syscall2(SYS_symlink, "d", D "/l"), 0);
    expect("setup_file_link", syscall2(SYS_symlink, "f", D "/lf"), 0);
    expect("setup_access_file", syscall5(SYS_setxattr, D "/f", ACCESS, acl, sizeof acl, 0), 0);
    expect("file_access", llist(D "/f"), 24);
    check("file_access_bytes", mem_eq(buf, ACCESS "\0", 24), 0);
    expect("file_link_empty", llist(D "/lf"), 0);
    expect("setup_access_dir", syscall5(SYS_setxattr, D "/d", ACCESS, acl, sizeof acl, 0), 0);
    expect("setup_default_dir", syscall5(SYS_setxattr, D "/d", DEFAULT, acl, sizeof acl, 0), 0);
    expect("dir_both", llist(D "/d"), 49);
    expect("dir_access", has(ACCESS), 1);
    expect("dir_default", has(DEFAULT), 1);
    expect("dir_link_empty", llist(D "/l"), 0);
    expect("dir_link_slash", llist(D "/l/"), 49);
    expect("dir_link_slash_default", has(DEFAULT), 1);
    expect("setup_subdir", a_mkdir(D "/d/sub", 0755), 0);
    expect("subdir_inherits", llist(D "/d/sub"), 49);
    expect("subdir_access", has(ACCESS), 1);
    expect("subdir_default", has(DEFAULT), 1);
    expect("setup_inner_link", syscall2(SYS_symlink, "sub", D "/d/ls"), 0);
    expect("inner_link_empty", llist(D "/d/ls"), 0);
    expect("cleanup_ls", a_unlink(D "/d/ls"), 0);
    expect("cleanup_sub", a_rmdir(D "/d/sub"), 0);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_lf", a_unlink(D "/lf"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_d", a_rmdir(D "/d"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
