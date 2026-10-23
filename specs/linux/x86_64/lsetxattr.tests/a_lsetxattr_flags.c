/* lsetxattr flags: 0 creates or replaces; XATTR_CREATE (1) fails with
 * EEXIST when the name exists; XATTR_REPLACE (2) fails with ENODATA when it
 * does not; both (3) never succeed. Any other bit is EINVAL. flags is an
 * int: bits above the low 32 are ignored. A failed call leaves the value
 * as it was. */
#include "a_sys.h"

#define SYS_lsetxattr 189
#define SYS_lgetxattr 192
#define ENODATA 61
#define D "/tmp/a_lsetxattr_flags"
#define F D "/f"

static char buf[64];

static i64 lset(const char *name, const char *v, i64 flags) {
    return syscall5(SYS_lsetxattr, F, name, v, str_len(v), flags);
}

static i64 get(const char *name) { return syscall4(SYS_lgetxattr, F, name, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    expect("replace_missing_enodata", lset("user.a", "1", 2), -ENODATA);
    expect("still_missing", get("user.a"), -ENODATA);
    expect("both_missing_enodata", lset("user.a", "1", 3), -ENODATA);
    expect("create_new", lset("user.a", "1", 1), 0);
    expect("create_existing_eexist", lset("user.a", "22", 1), -EEXIST);
    expect("value_kept", get("user.a"), 1);
    expect("both_existing_eexist", lset("user.a", "22", 3), -EEXIST);
    expect("replace_existing", lset("user.a", "333", 2), 0);
    expect("replaced_len", get("user.a"), 3);
    check("replaced_bytes", mem_eq(buf, "333", 3), 0);
    expect("flags0_replace", lset("user.a", "4444", 0), 0);
    expect("flags0_len", get("user.a"), 4);
    expect("flags4_einval", lset("user.b", "1", 4), -EINVAL);
    expect("flags5_einval", lset("user.b", "1", 5), -EINVAL);
    expect("flags8_einval", lset("user.b", "1", 8), -EINVAL);
    expect("flags_sign_einval", lset("user.b", "1", 0x80000000LL), -EINVAL);
    expect("flags_minus1_einval", lset("user.b", "1", -1), -EINVAL);
    expect("einval_not_created", get("user.b"), -ENODATA);
    expect("high_bits_ignored_create", lset("user.c", "1", 0x100000001LL), 0);
    expect("high_bits_ignored_eexist", lset("user.c", "1", 0x100000001LL), -EEXIST);
    expect("high_bits_ignored_zero", lset("user.c", "55", 0x100000000LL), 0);
    expect("high_bits_value", get("user.c"), 2);
    expect("cleanup_f", a_unlink(F), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
