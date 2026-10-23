/* lsetxattr (189) on regular files and directories behaves like setxattr:
 * flags 0 creates or replaces, the value is stored byte for byte (NUL
 * bytes included), a zero-length value may come with a NULL pointer, and
 * names on one inode are independent and case-sensitive. */
#include "a_sys.h"

#define SYS_lsetxattr 189
#define SYS_getxattr 191
#define SYS_lgetxattr 192
#define D "/tmp/a_lsetxattr_basic"

static char buf[64];

static i64 lset(const char *p, const char *name, const void *v, i64 size, i64 flags) {
    return syscall5(SYS_lsetxattr, p, name, v, size, flags);
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("set_new", lset(D "/f", "user.a", "hello", 5, 0), 0);
    expect("get_new", syscall4(SYS_getxattr, D "/f", "user.a", buf, sizeof buf), 5);
    check("get_new_bytes", mem_eq(buf, "hello", 5), 0);
    expect("replace_flags0", lset(D "/f", "user.a", "hi", 2, 0), 0);
    expect("get_replaced", syscall4(SYS_lgetxattr, D "/f", "user.a", buf, sizeof buf), 2);
    check("get_replaced_bytes", mem_eq(buf, "hi", 2), 0);
    expect("binary_value", lset(D "/f", "user.bin", "a\0b\0", 4, 0), 0);
    expect("binary_len", syscall4(SYS_getxattr, D "/f", "user.bin", buf, sizeof buf), 4);
    check("binary_bytes", mem_eq(buf, "a\0b\0", 4), 0);
    expect("empty_null_value", lset(D "/f", "user.empty", 0, 0, 0), 0);
    expect("empty_len", syscall4(SYS_getxattr, D "/f", "user.empty", buf, sizeof buf), 0);
    expect("upper_name", lset(D "/f", "user.A", "X", 1, 0), 0);
    expect("lower_kept", syscall4(SYS_getxattr, D "/f", "user.a", buf, sizeof buf), 2);
    expect("prefix_name", lset(D "/f", "user.a0", "123", 3, 0), 0);
    expect("prefix_independent", syscall4(SYS_getxattr, D "/f", "user.a", buf, sizeof buf), 2);
    expect("dir_set", lset(D "/sub", "user.d", "dd", 2, 0), 0);
    expect("dir_get", syscall4(SYS_getxattr, D "/sub", "user.d", buf, sizeof buf), 2);
    expect("dir_trailing_slash", lset(D "/sub/", "user.e", "e", 1, 0), 0);
    expect("dir_dot", lset(D "/sub/.", "user.f", "f", 1, 0), 0);
    expect("dir_dot_get", syscall4(SYS_getxattr, D "/sub", "user.f", buf, sizeof buf), 1);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
