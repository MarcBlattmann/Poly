/* removexattr basics: removing an existing user.* attribute returns 0;
 * afterwards getxattr gives ENODATA and a second removal ENODATA too. A
 * name that was never set is ENODATA. Other attributes of the file stay,
 * with their values; names are case-sensitive ("user.k" vs "user.K") and
 * exact ("user.k" does not remove "user.k0"). listxattr shrinks by the
 * removed name and its NUL. Works on directories too, and the same name
 * can be set again after removal (even with XATTR_CREATE). */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_getxattr 191
#define SYS_listxattr 194
#define SYS_removexattr 197
#define ENODATA 61
#define D "/tmp/a_removexattr_basic"

static char buf[256];

static i64 rm(const char *p, const char *name) { return syscall2(SYS_removexattr, p, name); }
static i64 set(const char *p, const char *name, const char *v, i64 flags) {
    return syscall5(SYS_setxattr, p, name, v, str_len(v), flags);
}
static i64 get(const char *p, const char *name) { return syscall4(SYS_getxattr, p, name, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);
    expect("set_k", set(D "/f", "user.k", "one", 0), 0);
    expect("set_K", set(D "/f", "user.K", "two!", 0), 0);
    expect("set_k0", set(D "/f", "user.k0", "three", 0), 0);
    expect("list_before", syscall3(SYS_listxattr, D "/f", buf, sizeof buf), 22);
    expect("remove_k", rm(D "/f", "user.k"), 0);
    expect("get_k_gone", get(D "/f", "user.k"), -ENODATA);
    expect("remove_k_again", rm(D "/f", "user.k"), -ENODATA);
    expect("get_K_stays", get(D "/f", "user.K"), 4);
    check("K_value", mem_eq(buf, "two!", 4), 0);
    expect("get_k0_stays", get(D "/f", "user.k0"), 5);
    check("k0_value", mem_eq(buf, "three", 5), 0);
    expect("list_after", syscall3(SYS_listxattr, D "/f", buf, sizeof buf), 15);
    expect("remove_never_set", rm(D "/f", "user.never"), -ENODATA);
    expect("remove_prefix_of_name", rm(D "/f", "user.k0x"), -ENODATA);
    expect("set_k_again_create", set(D "/f", "user.k", "new", 1), 0);
    expect("get_k_new", get(D "/f", "user.k"), 3);
    check("k_new_value", mem_eq(buf, "new", 3), 0);
    expect("remove_K", rm(D "/f", "user.K"), 0);
    expect("remove_k0", rm(D "/f", "user.k0"), 0);
    expect("remove_k_last", rm(D "/f", "user.k"), 0);
    expect("list_empty", syscall3(SYS_listxattr, D "/f", buf, sizeof buf), 0);
    expect("dir_set", set(D, "user.d", "x", 0), 0);
    expect("dir_remove", rm(D, "user.d"), 0);
    expect("dir_get_gone", get(D, "user.d"), -ENODATA);
    expect("dir_remove_again", rm(D, "user.d"), -ENODATA);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    expect("contents_unchanged", a_read(fd, buf, sizeof buf), 4);
    a_close(fd);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
