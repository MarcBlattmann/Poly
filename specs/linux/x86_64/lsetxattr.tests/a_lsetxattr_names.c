/* Attribute names for lsetxattr on a regular file: 1 to 255 bytes, prefix
 * included; any bytes after "user." are allowed. Empty or 256+ bytes:
 * ERANGE (also when the name has no NUL within its first 256 readable
 * bytes); unreadable: EFAULT. An unknown or missing prefix is EOPNOTSUPP;
 * "user." alone is EINVAL. */
#include "a_sys.h"

#define SYS_lsetxattr 189
#define SYS_getxattr 191
#define D "/tmp/a_lsetxattr_names"
#define F D "/f"

static char buf[64];
static char name[300];

static i64 lset(const char *n) { return syscall5(SYS_lsetxattr, F, n, "v", 1, 0); }

static void fill_name(int len) {
    name[0] = 'u', name[1] = 's', name[2] = 'e', name[3] = 'r', name[4] = '.';
    for (int i = 5; i < 299; i++) name[i] = 'n';
    name[len] = 0;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    fill_name(255);
    expect("name255_ok", lset(name), 0);
    expect("name255_get", syscall4(SYS_getxattr, F, name, buf, sizeof buf), 1);
    fill_name(256);
    expect("name256_erange", lset(name), -ERANGE);
    name[256] = 'n';
    name[299] = 0;
    expect("name299_erange", lset(name), -ERANGE);
    expect("empty_erange", lset(""), -ERANGE);
    expect("null_efault", lset(0), -EFAULT);
    expect("bad_ptr_efault", syscall5(SYS_lsetxattr, F, 8, "v", 1, 0), -EFAULT);
    expect("short_ok", lset("user.x"), 0);
    expect("space_ok", lset("user. "), 0);
    expect("dots_ok", lset("user.."), 0);
    expect("slash_ok", lset("user.a/b"), 0);
    expect("high_byte_ok", lset("user.\xff\x01"), 0);
    expect("high_byte_get", syscall4(SYS_getxattr, F, "user.\xff\x01", buf, sizeof buf), 1);
    expect("user_dot_einval", lset("user."), -EINVAL);
    expect("user_no_dot_eopnotsupp", lset("user"), -EOPNOTSUPP);
    expect("upper_eopnotsupp", lset("USER.a"), -EOPNOTSUPP);
    expect("leading_dot_eopnotsupp", lset(".user.a"), -EOPNOTSUPP);
    expect("unknown_eopnotsupp", lset("foo.bar"), -EOPNOTSUPP);
    expect("plain_eopnotsupp", lset("plain"), -EOPNOTSUPP);
    expect("system_unknown_eopnotsupp", lset("system.x"), -EOPNOTSUPP);
    expect("cleanup_f", a_unlink(F), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
