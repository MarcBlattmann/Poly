/* Attributes belong to the inode: lgetxattr through every hard link reads
 * the same value, a rename keeps it, a value set through one link reads
 * back through another, and each file and directory has its own set (a
 * new file under an old name starts with none). Replacing a file by a
 * symbolic link under the same name: lgetxattr then reads the link
 * (ENODATA), while the old file, still linked elsewhere, keeps its value.
 * Renaming a symbolic link keeps it a link. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_getxattr 191
#define SYS_lgetxattr 192
#define ENODATA 61
#define D "/tmp/a_lgetxattr_inode"

static char buf[64];
static char name[16];

static i64 lget(const char *p, const char *n) { return syscall4(SYS_lgetxattr, p, n, buf, sizeof buf); }

static i64 set(const char *p, const char *n, const char *v) {
    return syscall5(SYS_setxattr, p, n, v, str_len(v), 0);
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_a", a_make_file(D "/a", "", 0644), 0);
    expect("setup_b", a_make_file(D "/b", "", 0644), 0);
    expect("set_a", set(D "/a", "user.k", "on-a"), 0);
    expect("set_b", set(D "/b", "user.k", "b"), 0);
    expect("get_a", lget(D "/a", "user.k"), 4);
    check("a_value", mem_eq(buf, "on-a", 4), 0);
    expect("get_b", lget(D "/b", "user.k"), 1);
    expect("link", syscall2(SYS_link, D "/a", D "/a2"), 0);
    expect("get_via_link", lget(D "/a2", "user.k"), 4);
    expect("set_via_link", set(D "/a2", "user.k", "new"), 0);
    expect("get_original", lget(D "/a", "user.k"), 3);
    check("original_value", mem_eq(buf, "new", 3), 0);
    expect("rename", syscall2(SYS_rename, D "/a", D "/c"), 0);
    expect("get_renamed", lget(D "/c", "user.k"), 3);
    expect("rename_over_b", syscall2(SYS_rename, D "/c", D "/b"), 0);
    expect("get_replaced_name", lget(D "/b", "user.k"), 3);
    check("replaced_name_value", mem_eq(buf, "new", 3), 0);
    expect("setup_sym", syscall2(SYS_symlink, "a2", D "/s"), 0);
    expect("sym_over_b", syscall2(SYS_rename, D "/s", D "/b"), 0);
    expect("name_now_link", lget(D "/b", "user.k"), -ENODATA);
    expect("getxattr_through_it", syscall4(SYS_getxattr, D "/b", "user.k", buf, sizeof buf), 3);
    expect("old_file_keeps_value", lget(D "/a2", "user.k"), 3);
    expect("rename_link", syscall2(SYS_rename, D "/b", D "/s2"), 0);
    expect("renamed_link_still_link", lget(D "/s2", "user.k"), -ENODATA);
    expect("unlink_s2", a_unlink(D "/s2"), 0);
    expect("new_file_old_name", a_make_file(D "/b", "", 0644), 0);
    expect("new_file_empty", lget(D "/b", "user.k"), -ENODATA);
    str_cpy(name, "user.n0");
    int ok = 1;
    for (int i = 0; i < 10; i++) {
        name[6] = (char)('0' + i);
        char v[2] = { (char)('a' + i), 0 };
        if (set(D "/b", name, v) != 0) ok = 0;
    }
    check("set_ten_names", ok, 0);
    ok = 1;
    for (int i = 9; i >= 0; i--) {
        name[6] = (char)('0' + i);
        if (lget(D "/b", name) != 1 || buf[0] != (char)('a' + i)) ok = 0;
    }
    check("ten_names_read_back", ok, 0);
    expect("name_prefix_not_matched", lget(D "/b", "user.n"), -ENODATA);
    expect("set_dir", set(D, "user.k", "dir"), 0);
    expect("get_dir", lget(D, "user.k"), 3);
    expect("file_unchanged", lget(D "/a2", "user.k"), 3);
    check("file_value", mem_eq(buf, "new", 3), 0);
    expect("cleanup_a2", a_unlink(D "/a2"), 0);
    expect("cleanup_b", a_unlink(D "/b"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
