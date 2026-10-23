/* setxattr names: empty ERANGE; 255 bytes (prefix included) is the longest
 * accepted, 256 ERANGE; a name needs a known prefix ("user." etc.), else
 * EOPNOTSUPP ("user" without the dot, "USER.", ".user.", "foo.bar", unknown
 * "system." names); "user." with nothing after it is EINVAL. Rejected
 * names create nothing. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_getxattr 191
#define ENODATA 61
#define D "/tmp/a_setxattr_names"

static char name[400];
static char buf[64];

/* "user." followed by 'n' up to a total length of `len`. */
static const char *long_name(int len) {
    str_cpy(name, "user.");
    for (int i = 5; i < len; i++) name[i] = 'n';
    name[len] = 0;
    return name;
}

static i64 set(const char *n) { return syscall5(SYS_setxattr, D "/f", n, "v", 1, 0); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("empty_name_erange", set(""), -ERANGE);
    expect("name_255_ok", set(long_name(255)), 0);
    expect("name_255_readable", syscall4(SYS_getxattr, D "/f", long_name(255), buf, sizeof buf), 1);
    expect("name_256_erange", set(long_name(256)), -ERANGE);
    expect("name_399_erange", set(long_name(399)), -ERANGE);
    expect("short_name_ok", set("user.x"), 0);
    expect("no_dot_eopnotsupp", set("user"), -EOPNOTSUPP);
    expect("no_prefix_eopnotsupp", set("plain"), -EOPNOTSUPP);
    expect("upper_prefix_eopnotsupp", set("USER.x"), -EOPNOTSUPP);
    expect("leading_dot_eopnotsupp", set(".user.x"), -EOPNOTSUPP);
    expect("unknown_ns_eopnotsupp", set("foo.bar"), -EOPNOTSUPP);
    expect("unknown_system_eopnotsupp", set("system.x"), -EOPNOTSUPP);
    expect("system_dot_eopnotsupp", set("system."), -EOPNOTSUPP);
    expect("user_dot_only_einval", set("user."), -EINVAL);
    expect("user_dot_dot_ok", set("user.."), 0);
    expect("user_space_ok", set("user. "), 0);
    expect("rejected_not_created", syscall4(SYS_getxattr, D "/f", "user.nn", buf, sizeof buf), -ENODATA);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
