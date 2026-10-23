/* Attributes belong to the inode: getxattr through every hard link reads
 * the same value, a rename keeps it, a value set through one link reads
 * back through another, and each file and directory has its own set (the
 * same name on two files holds two values; a new file under an old name
 * starts with none). Many names on one inode are independent. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_getxattr 191
#define ENODATA 61
#define D "/tmp/a_getxattr_inode"

static char buf[64];
static char name[16];

static i64 get(const char *p, const char *n) { return syscall4(SYS_getxattr, p, n, buf, sizeof buf); }

static i64 set(const char *p, const char *n, const char *v) {
    return syscall5(SYS_setxattr, p, n, v, str_len(v), 0);
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_a", a_make_file(D "/a", "", 0644), 0);
    expect("setup_b", a_make_file(D "/b", "", 0644), 0);
    expect("set_a", set(D "/a", "user.k", "on-a"), 0);
    expect("set_b", set(D "/b", "user.k", "b"), 0);
    expect("get_a", get(D "/a", "user.k"), 4);
    check("a_value", mem_eq(buf, "on-a", 4), 0);
    expect("get_b", get(D "/b", "user.k"), 1);
    expect("link", syscall2(SYS_link, D "/a", D "/a2"), 0);
    expect("get_via_link", get(D "/a2", "user.k"), 4);
    expect("set_via_link", set(D "/a2", "user.k", "new"), 0);
    expect("get_original", get(D "/a", "user.k"), 3);
    check("original_value", mem_eq(buf, "new", 3), 0);
    expect("rename", syscall2(SYS_rename, D "/a", D "/c"), 0);
    expect("get_renamed", get(D "/c", "user.k"), 3);
    expect("rename_over_b", syscall2(SYS_rename, D "/c", D "/b"), 0);
    expect("get_replaced_name", get(D "/b", "user.k"), 3);
    check("replaced_name_value", mem_eq(buf, "new", 3), 0);
    expect("unlink_b", a_unlink(D "/b"), 0);
    expect("new_file_old_name", a_make_file(D "/b", "", 0644), 0);
    expect("new_file_empty", get(D "/b", "user.k"), -ENODATA);
    expect("still_on_link", get(D "/a2", "user.k"), 3);
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
        if (get(D "/b", name) != 1 || buf[0] != (char)('a' + i)) ok = 0;
    }
    check("ten_names_read_back", ok, 0);
    expect("name_prefix_not_matched", get(D "/b", "user.n"), -ENODATA);
    expect("set_dir", set(D, "user.k", "dir"), 0);
    expect("get_dir", get(D, "user.k"), 3);
    expect("file_unchanged", get(D "/a2", "user.k"), 3);
    check("file_value", mem_eq(buf, "new", 3), 0);
    expect("cleanup_a2", a_unlink(D "/a2"), 0);
    expect("cleanup_b", a_unlink(D "/b"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
