/* The list belongs to the inode. A hard link to a file lists the file's
 * names, also after the first name is renamed or removed; names added
 * through one link show up through the other. A hard link to a symbolic
 * link is that symbolic link (llistxattr lists it as empty, listxattr
 * follows it to the file). A renamed symbolic link still lists as empty.
 */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_listxattr 194
#define SYS_llistxattr 195
#define D "/tmp/a_llistxattr_inode"

static char buf[256];
static i64 len;

static i64 llist(const char *p) { return len = syscall3(SYS_llistxattr, p, buf, sizeof buf); }

static int has(const char *name) {
    int n = 0;
    for (i64 i = 0; i < len; i += str_len(buf + i) + 1)
        if (str_len(buf + i) == str_len(name) && mem_eq(buf + i, name, str_len(name))) n++;
    return n;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, D "/f", "user.one", "1", 1, 0), 0);
    expect("setup_hard", syscall5(SYS_linkat, AT_FDCWD, D "/f", AT_FDCWD, D "/h", 0), 0);
    expect("hard_link", llist(D "/h"), 9);
    check("hard_link_bytes", mem_eq(buf, "user.one\0", 9), 0);
    expect("setup_set_via_hard", syscall5(SYS_setxattr, D "/h", "user.two", "2", 1, 0), 0);
    expect("first_sees_both", llist(D "/f"), 18);
    expect("first_one", has("user.one"), 1);
    expect("first_two", has("user.two"), 1);
    expect("setup_rename", syscall2(SYS_rename, D "/f", D "/g"), 0);
    expect("renamed", llist(D "/g"), 18);
    expect("renamed_old_name", llist(D "/f"), -ENOENT);
    expect("setup_symlink", syscall2(SYS_symlink, "g", D "/s"), 0);
    expect("setup_hard_symlink", syscall5(SYS_linkat, AT_FDCWD, D "/s", AT_FDCWD, D "/hs", 0), 0);
    expect("hard_symlink", llist(D "/hs"), 0);
    expect("hard_symlink_listxattr", syscall3(SYS_listxattr, D "/hs", buf, sizeof buf), 18);
    expect("setup_rename_symlink", syscall2(SYS_rename, D "/s", D "/s2"), 0);
    expect("renamed_symlink", llist(D "/s2"), 0);
    expect("setup_unlink_g", a_unlink(D "/g"), 0);
    expect("after_unlink", llist(D "/h"), 18);
    expect("after_unlink_two", has("user.two"), 1);
    expect("cleanup_h", a_unlink(D "/h"), 0);
    expect("cleanup_hs", a_unlink(D "/hs"), 0);
    expect("cleanup_s2", a_unlink(D "/s2"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
