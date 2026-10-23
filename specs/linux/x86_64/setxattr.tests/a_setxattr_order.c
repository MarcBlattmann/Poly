/* setxattr error order: with several problems the first of these decides:
 * flags (EINVAL), the name (EFAULT, ERANGE for empty or too long), the
 * value size (E2BIG), the value (EFAULT), then the path (EFAULT, ENOENT,
 * ...), and only then the namespace and the attribute (EOPNOTSUPP, EINVAL
 * for "user.", EPERM for trusted.*, EEXIST, ENODATA). fsetxattr: flags,
 * name, size and value come before the descriptor (EBADF), the namespace
 * after it. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_fsetxattr 190
#define D "/tmp/a_setxattr_order"
#define NOPE D "/nope"
#define BAD ((const void *)8)

static char big[65540];
static char longname[300];

static i64 set(const void *path, const void *name, const void *v, i64 size, i64 flags) {
    return syscall5(SYS_setxattr, path, name, v, size, flags);
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    str_cpy(longname, "user.");
    for (int i = 5; i < 299; i++) longname[i] = 'l';
    expect("flags_before_bad_name", set(D "/f", BAD, "v", 1, 8), -EINVAL);
    expect("flags_before_long_name", set(D "/f", longname, "v", 1, 4), -EINVAL);
    expect("flags_before_size", set(D "/f", "user.a", big, 65537, 4), -EINVAL);
    expect("flags_before_path", set(NOPE, "user.a", "v", 1, 8), -EINVAL);
    expect("flags_before_bad_path", set(BAD, "user.a", "v", 1, 8), -EINVAL);
    expect("empty_name_before_size", set(D "/f", "", big, 65537, 0), -ERANGE);
    expect("bad_name_before_size", set(D "/f", BAD, big, 65537, 0), -EFAULT);
    expect("empty_name_before_value", set(D "/f", "", BAD, 1, 0), -ERANGE);
    expect("long_name_before_path", set(NOPE, longname, "v", 1, 0), -ERANGE);
    expect("empty_name_before_bad_path", set(BAD, "", "v", 1, 0), -ERANGE);
    expect("size_before_value", set(D "/f", "user.a", BAD, 65537, 0), -E2BIG);
    expect("size_before_path", set(NOPE, "user.a", big, 65537, 0), -E2BIG);
    expect("size_before_bad_path", set(BAD, "user.a", big, 65537, 0), -E2BIG);
    expect("value_before_path", set(NOPE, "user.a", BAD, 1, 0), -EFAULT);
    expect("value_before_empty_path", set("", "user.a", BAD, 1, 0), -EFAULT);
    expect("path_before_namespace", set(NOPE, "foo.bar", "v", 1, 0), -ENOENT);
    expect("path_before_user_dot", set(NOPE, "user.", "v", 1, 0), -ENOENT);
    expect("path_before_trusted", set(NOPE, "trusted.a", "v", 1, 0), -ENOENT);
    expect("path_before_replace", set(NOPE, "user.a", "v", 1, 2), -ENOENT);
    expect("enotdir_before_namespace", set(D "/f/x", "foo.bar", "v", 1, 0), -ENOTDIR);
    expect("fset_flags_before_fd", syscall5(SYS_fsetxattr, 99, "user.a", "v", 1, 8), -EINVAL);
    expect("fset_name_before_fd", syscall5(SYS_fsetxattr, 99, "", "v", 1, 0), -ERANGE);
    expect("fset_size_before_fd", syscall5(SYS_fsetxattr, 99, "user.a", big, 65537, 0), -E2BIG);
    expect("fset_value_before_fd", syscall5(SYS_fsetxattr, 99, "user.a", BAD, 1, 0), -EFAULT);
    expect("fset_fd_before_namespace", syscall5(SYS_fsetxattr, 99, "foo.a", "v", 1, 0), -EBADF);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
