/* lgetxattr error order: with several problems the first of these
 * decides: the name (EFAULT, ERANGE for empty or too long), then the path
 * (EFAULT, ENOENT, ENOTDIR, ...), then the file type (user.* on a
 * symbolic link: ENODATA; ACL names on a link: EOPNOTSUPP; "security."
 * EINVAL on any type), then the namespace (EOPNOTSUPP, EINVAL for
 * "user."), then whether the attribute exists (ENODATA), then the size
 * (ERANGE), and only then the buffer (EFAULT). On a link the size and
 * buffer are never looked at. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_lgetxattr 192
#define ENODATA 61
#define D "/tmp/a_lgetxattr_order"
#define NOPE D "/nope"
#define BAD ((void *)8)

static char buf[64];
static char longname[300];

static i64 lget(const void *p, const void *name, void *b, i64 size) {
    return syscall4(SYS_lgetxattr, p, name, b, size);
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, D "/f", "user.a", "value", 5, 0), 0);
    expect("setup_link", syscall2(SYS_symlink, "f", D "/l"), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "nope", D "/dangling"), 0);
    str_cpy(longname, "user.");
    for (int i = 5; i < 299; i++) longname[i] = 'l';
    expect("bad_name_before_missing_path", lget(NOPE, BAD, buf, 10), -EFAULT);
    expect("bad_name_before_bad_path", lget(BAD, BAD, buf, 10), -EFAULT);
    expect("bad_name_on_link", lget(D "/l", BAD, buf, 10), -EFAULT);
    expect("empty_name_before_missing_path", lget(NOPE, "", buf, 10), -ERANGE);
    expect("empty_name_before_bad_path", lget(BAD, "", buf, 10), -ERANGE);
    expect("empty_name_on_link", lget(D "/l", "", buf, 10), -ERANGE);
    expect("long_name_before_empty_path", lget("", longname, buf, 10), -ERANGE);
    expect("long_name_on_link", lget(D "/l", longname, buf, 10), -ERANGE);
    expect("path_before_unknown_prefix", lget(NOPE, "foo.bar", buf, 10), -ENOENT);
    expect("path_before_user_dot", lget(NOPE, "user.", buf, 10), -ENOENT);
    expect("path_before_security_dot", lget(NOPE, "security.", buf, 10), -ENOENT);
    expect("path_before_bad_buffer", lget(NOPE, "user.a", BAD, 10), -ENOENT);
    expect("bad_path_before_unknown_prefix", lget(BAD, "foo.bar", buf, 10), -EFAULT);
    expect("enotdir_before_unknown_prefix", lget(D "/f/x", "foo.bar", buf, 10), -ENOTDIR);
    expect("dangling_slash_before_prefix", lget(D "/dangling/", "foo.bar", buf, 10), -ENOENT);
    expect("link_user_dot_enodata", lget(D "/l", "user.", buf, 10), -ENODATA);
    expect("link_security_dot_einval", lget(D "/l", "security.", BAD, 10), -EINVAL);
    expect("link_unknown_eopnotsupp", lget(D "/l", "foo.bar", BAD, 10), -EOPNOTSUPP);
    expect("link_acl_eopnotsupp", lget(D "/l", "system.posix_acl_access", BAD, 1), -EOPNOTSUPP);
    expect("link_before_small_size", lget(D "/l", "user.a", buf, 1), -ENODATA);
    expect("link_before_bad_buffer", lget(D "/l", "user.a", BAD, 10), -ENODATA);
    expect("unknown_prefix_before_buffer", lget(D "/f", "foo.bar", BAD, 10), -EOPNOTSUPP);
    expect("user_dot_before_buffer", lget(D "/f", "user.", BAD, 10), -EINVAL);
    expect("missing_before_bad_buffer", lget(D "/f", "user.b", BAD, 10), -ENODATA);
    expect("missing_before_small_size", lget(D "/f", "user.b", buf, 1), -ENODATA);
    expect("small_size_before_bad_buffer", lget(D "/f", "user.a", BAD, 4), -ERANGE);
    expect("small_size_before_null", lget(D "/f", "user.a", 0, 4), -ERANGE);
    expect("bad_buffer_last", lget(D "/f", "user.a", BAD, 5), -EFAULT);
    expect("good_call", lget(D "/f", "user.a", buf, 5), 5);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_dangling", a_unlink(D "/dangling"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
