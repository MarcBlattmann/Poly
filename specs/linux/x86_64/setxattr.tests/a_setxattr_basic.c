/* setxattr: sets a user.* attribute on a file and a directory; the value is
 * stored byte for byte (NUL bytes included) and a later set replaces it;
 * a zero-length value (even with a NULL pointer) is allowed; names are
 * case-sensitive and may hold any bytes after the prefix. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_getxattr 191
#define D "/tmp/a_setxattr_basic"

static char buf[256];

/* The value of `name` on `path` equals the `n` bytes at `want`. */
static int has(const char *path, const char *name, const char *want, i64 n) {
    i64 r = syscall4(SYS_getxattr, path, name, buf, sizeof buf);
    return r == n && mem_eq(buf, want, (size_t)n);
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);
    expect("set_new", syscall5(SYS_setxattr, D "/f", "user.a", "xyz", 3, 0), 0);
    check("value_stored", has(D "/f", "user.a", "xyz", 3), 0);
    expect("set_binary_replaces", syscall5(SYS_setxattr, D "/f", "user.a", "hello\0world\0", 12, 0), 0);
    check("binary_value_kept", has(D "/f", "user.a", "hello\0world\0", 12), 0);
    expect("set_shorter", syscall5(SYS_setxattr, D "/f", "user.a", "z", 1, 0), 0);
    check("shorter_value", has(D "/f", "user.a", "z", 1), 0);
    expect("set_empty_null_value", syscall5(SYS_setxattr, D "/f", "user.e", 0, 0, 0), 0);
    expect("empty_value_length", syscall4(SYS_getxattr, D "/f", "user.e", buf, sizeof buf), 0);
    expect("set_empty_over_value", syscall5(SYS_setxattr, D "/f", "user.a", "ignored", 0, 0), 0);
    expect("emptied_length", syscall4(SYS_getxattr, D "/f", "user.a", buf, sizeof buf), 0);
    expect("set_upper_case_name", syscall5(SYS_setxattr, D "/f", "user.A", "BIG", 3, 0), 0);
    check("upper_value", has(D "/f", "user.A", "BIG", 3), 0);
    expect("lower_untouched", syscall4(SYS_getxattr, D "/f", "user.a", buf, sizeof buf), 0);
    expect("set_odd_name", syscall5(SYS_setxattr, D "/f", "user.a b/c..", "1", 1, 0), 0);
    check("odd_name_value", has(D "/f", "user.a b/c..", "1", 1), 0);
    expect("set_high_bytes_name", syscall5(SYS_setxattr, D "/f", "user.\xff\x01", "2", 1, 0), 0);
    check("high_bytes_value", has(D "/f", "user.\xff\x01", "2", 1), 0);
    expect("set_dir", syscall5(SYS_setxattr, D, "user.d", "dir", 3, 0), 0);
    check("dir_value", has(D, "user.d", "dir", 3), 0);
    expect("set_dir_slash", syscall5(SYS_setxattr, D "/", "user.d", "d2", 2, 0), 0);
    check("dir_slash_value", has(D, "user.d", "d2", 2), 0);
    expect("set_dot", syscall5(SYS_setxattr, D "/.", "user.d", "d3", 2, 0), 0);
    check("dot_value", has(D, "user.d", "d3", 2), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
