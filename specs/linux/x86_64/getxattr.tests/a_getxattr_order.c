/* getxattr error order: with several problems the first of these decides:
 * the name (EFAULT, ERANGE for empty or too long), then the path (EFAULT,
 * ENOENT, ENOTDIR, ...), then the namespace and permission (EOPNOTSUPP,
 * EINVAL for "user.", EACCES), then whether the attribute exists
 * (ENODATA), then the size (ERANGE), and only then the buffer (EFAULT).
 * The size and buffer are never looked at for a missing attribute. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_getxattr 191
#define ENODATA 61
#define D "/tmp/a_getxattr_order"
#define NOPE D "/nope"
#define BAD ((void *)8)

static char buf[64];
static char longname[300];

static i64 get(const void *p, const void *name, void *b, i64 size) { return syscall4(SYS_getxattr, p, name, b, size); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, D "/f", "user.a", "value", 5, 0), 0);
    str_cpy(longname, "user.");
    for (int i = 5; i < 299; i++) longname[i] = 'l';
    expect("bad_name_before_missing_path", get(NOPE, BAD, buf, 10), -EFAULT);
    expect("bad_name_before_bad_path", get(BAD, BAD, buf, 10), -EFAULT);
    expect("empty_name_before_missing_path", get(NOPE, "", buf, 10), -ERANGE);
    expect("empty_name_before_bad_path", get(BAD, "", buf, 10), -ERANGE);
    expect("long_name_before_missing_path", get(NOPE, longname, buf, 10), -ERANGE);
    expect("long_name_before_empty_path", get("", longname, buf, 10), -ERANGE);
    expect("path_before_unknown_prefix", get(NOPE, "foo.bar", buf, 10), -ENOENT);
    expect("path_before_user_dot", get(NOPE, "user.", buf, 10), -ENOENT);
    expect("path_before_system_x", get(NOPE, "system.x", buf, 10), -ENOENT);
    expect("path_before_bad_buffer", get(NOPE, "user.a", BAD, 10), -ENOENT);
    expect("bad_path_before_unknown_prefix", get(BAD, "foo.bar", buf, 10), -EFAULT);
    expect("enotdir_before_unknown_prefix", get(D "/f/x", "foo.bar", buf, 10), -ENOTDIR);
    expect("unknown_prefix_before_buffer", get(D "/f", "foo.bar", BAD, 10), -EOPNOTSUPP);
    expect("user_dot_before_buffer", get(D "/f", "user.", BAD, 10), -EINVAL);
    expect("missing_before_bad_buffer", get(D "/f", "user.b", BAD, 10), -ENODATA);
    expect("missing_before_small_size", get(D "/f", "user.b", buf, 1), -ENODATA);
    expect("small_size_before_bad_buffer", get(D "/f", "user.a", BAD, 4), -ERANGE);
    expect("small_size_before_null", get(D "/f", "user.a", 0, 4), -ERANGE);
    expect("bad_buffer_last", get(D "/f", "user.a", BAD, 5), -EFAULT);
    expect("good_call", get(D "/f", "user.a", buf, 5), 5);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
