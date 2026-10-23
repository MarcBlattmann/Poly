/* Error order of lsetxattr: flags (EINVAL), then the name (EFAULT,
 * ERANGE), then the size (E2BIG), then the value (EFAULT), then the path
 * walk; only after the path come the checks on the file found: on a
 * symbolic link user.* is EPERM; on a read-only file user.* and unknown
 * prefixes are EACCES, then the prefix (EOPNOTSUPP, "user." EINVAL) and
 * last EEXIST / ENODATA from the flags. Runs with no capabilities. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_lsetxattr 189
#define ENODATA 61
#define D "/tmp/a_lsetxattr_order"

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 lset(const char *p, const char *n, const void *v, i64 size, i64 flags) {
    return syscall5(SYS_lsetxattr, p, n, v, size, flags);
}

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_ro", a_make_file(D "/ro", "", 0444), 0);
    expect("setup_f", a_make_file(D "/f", "", 0644), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    expect("setup_existing", lset(D "/f", "user.e", "v", 1, 0), 0);
    /* Everything wrong at once: flags win. */
    expect("flags_first", lset(0, 0, 0, 65537, 8), -EINVAL);
    expect("flags_before_missing_path", lset(D "/nope", "user.a", "v", 1, 8), -EINVAL);
    expect("name_efault_before_size", lset(0, 0, 0, 65537, 0), -EFAULT);
    expect("name_erange_before_size", lset(0, "", 0, 65537, 0), -ERANGE);
    expect("size_before_value", lset(0, "user.a", 0, 65537, 0), -E2BIG);
    expect("value_before_path", lset(0, "user.a", 0, 1, 0), -EFAULT);
    expect("value_before_missing_path", lset(D "/nope", "user.a", 0, 1, 0), -EFAULT);
    expect("path_efault", lset(0, "user.a", "v", 1, 0), -EFAULT);
    expect("path_before_unknown_prefix", lset(D "/nope", "foo.bar", "v", 1, 0), -ENOENT);
    expect("path_before_user_dot", lset(D "/nope", "user.", "v", 1, 0), -ENOENT);
    expect("path_before_trusted", lset(D "/nope", "trusted.a", "v", 1, 0), -ENOENT);
    expect("path_before_flags_state", lset(D "/nope", "user.a", "v", 1, 2), -ENOENT);
    expect("link_eperm_before_replace", lset(D "/l", "user.a", "v", 1, 2), -EPERM);
    expect("link_eperm_before_user_dot", lset(D "/l", "user.", "v", 1, 0), -EPERM);
    expect("link_erange_before_eperm", lset(D "/l", "", "v", 1, 0), -ERANGE);
    expect("link_einval_flags_before_eperm", lset(D "/l", "user.a", "v", 1, 4), -EINVAL);
    expect("ro_eacces_before_replace", lset(D "/ro", "user.a", "v", 1, 2), -EACCES);
    expect("ro_eacces_before_user_dot", lset(D "/ro", "user.", "v", 1, 0), -EACCES);
    expect("ro_eacces_before_unknown", lset(D "/ro", "foo.a", "v", 1, 0), -EACCES);
    expect("ro_trusted_eperm_first", lset(D "/ro", "trusted.a", "v", 1, 0), -EPERM);
    expect("user_dot_before_state", lset(D "/f", "user.", "v", 1, 2), -EINVAL);
    expect("unknown_before_state", lset(D "/f", "foo.a", "v", 1, 1), -EOPNOTSUPP);
    expect("state_eexist_last", lset(D "/f", "user.e", "v", 1, 1), -EEXIST);
    expect("state_enodata_last", lset(D "/f", "user.m", "v", 1, 2), -ENODATA);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_ro", a_unlink(D "/ro"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
