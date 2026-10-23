/* getxattr of a name the file does not have is ENODATA: on a fresh file
 * and directory, for a name that differs only in case or length from an
 * existing one, after the attribute was removed (removexattr), and whatever
 * the buffer (size 0, NULL or an unmapped address with a size). The
 * POSIX ACL names read ENODATA when no ACL is set (also
 * system.posix_acl_default on a file). */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_getxattr 191
#define SYS_removexattr 197
#define ENODATA 61
#define D "/tmp/a_getxattr_missing"
#define BAD ((void *)8)

static char buf[64];

static i64 get(const char *p, const char *name, void *b, i64 size) { return syscall4(SYS_getxattr, p, name, b, size); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);
    expect("fresh_file", get(D "/f", "user.a", buf, sizeof buf), -ENODATA);
    expect("fresh_dir", get(D, "user.a", buf, sizeof buf), -ENODATA);
    expect("size0", get(D "/f", "user.a", 0, 0), -ENODATA);
    expect("null_buffer", get(D "/f", "user.a", 0, 10), -ENODATA);
    expect("bad_buffer", get(D "/f", "user.a", BAD, 10), -ENODATA);
    expect("acl_access", get(D "/f", "system.posix_acl_access", buf, sizeof buf), -ENODATA);
    expect("acl_default_file", get(D "/f", "system.posix_acl_default", buf, sizeof buf), -ENODATA);
    expect("acl_default_dir", get(D, "system.posix_acl_default", buf, sizeof buf), -ENODATA);
    expect("acl_access_dir", get(D, "system.posix_acl_access", buf, sizeof buf), -ENODATA);
    expect("setup_set", syscall5(SYS_setxattr, D "/f", "user.key", "v", 1, 0), 0);
    expect("present", get(D "/f", "user.key", buf, sizeof buf), 1);
    expect("other_case", get(D "/f", "user.Key", buf, sizeof buf), -ENODATA);
    expect("shorter_name", get(D "/f", "user.ke", buf, sizeof buf), -ENODATA);
    expect("longer_name", get(D "/f", "user.keys", buf, sizeof buf), -ENODATA);
    expect("remove", syscall2(SYS_removexattr, D "/f", "user.key"), 0);
    expect("removed", get(D "/f", "user.key", buf, sizeof buf), -ENODATA);
    expect("removed_size0", get(D "/f", "user.key", 0, 0), -ENODATA);
    expect("setup_set_again", syscall5(SYS_setxattr, D "/f", "user.key", "vv", 2, 0), 0);
    expect("set_again", get(D "/f", "user.key", buf, sizeof buf), 2);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
