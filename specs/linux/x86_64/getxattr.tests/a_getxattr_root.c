/* getxattr as root: with CAP_SYS_ADMIN trusted.* attributes read back
 * like user.* ones and "trusted." is EINVAL; root reads user.* from a file
 * whose mode gives no read permission (0000) and from another user's 0600
 * file. root may also set security.* names, which then read back. After
 * all capabilities are dropped, the same trusted.* attribute reads as
 * ENODATA (also with size 0) and "trusted." as ENODATA, the 0000 and the
 * other user's files are EACCES for user.*, while the security.* value
 * still reads back from the 0000 file (no read permission needed).
 * poly-diff: needs=root (trusted.* and security.*, chown, DAC override)
 */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_getxattr 191
#define ENODATA 61
#define D "/tmp/a_getxattr_root"
#define OTHER 12345

static char buf[64];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 set(const char *p, const char *name, const char *v) {
    return syscall5(SYS_setxattr, p, name, v, str_len(v), 0);
}

static i64 get(const char *p, const char *name) { return syscall4(SYS_getxattr, p, name, buf, sizeof buf); }

int test_main(void) {
    i64 uid = syscall0(SYS_geteuid);
    expect("is_root", uid, 0);
    if (uid != 0) return finish();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_none", a_make_file(D "/none", "", 0644), 0);
    expect("setup_other", a_make_file(D "/other", "", 0600), 0);
    expect("setup_set_trusted", set(D "/f", "trusted.t", "tt"), 0);
    expect("setup_set_none", set(D "/none", "user.a", "n"), 0);
    expect("setup_set_none_sec", set(D "/none", "security.poly", "sec"), 0);
    expect("setup_set_other", set(D "/other", "user.a", "o"), 0);
    expect("setup_none_mode", syscall3(SYS_fchmodat, AT_FDCWD, D "/none", 0), 0);
    expect("setup_other_own", syscall5(SYS_fchownat, AT_FDCWD, D "/other", OTHER, OTHER, 0), 0);
    expect("trusted_read", get(D "/f", "trusted.t"), 2);
    check("trusted_value", mem_eq(buf, "tt", 2), 0);
    expect("trusted_size0", syscall4(SYS_getxattr, D "/f", "trusted.t", 0, 0), 2);
    expect("trusted_missing", get(D "/f", "trusted.m"), -ENODATA);
    expect("trusted_dot_einval", get(D "/f", "trusted."), -EINVAL);
    expect("mode_0000_read", get(D "/none", "user.a"), 1);
    expect("other_0600_read", get(D "/other", "user.a"), 1);
    expect("security_read", get(D "/none", "security.poly"), 3);
    check("security_value", mem_eq(buf, "sec", 3), 0);
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped == 0) {
        expect("dropped_trusted_enodata", get(D "/f", "trusted.t"), -ENODATA);
        expect("dropped_trusted_size0", syscall4(SYS_getxattr, D "/f", "trusted.t", 0, 0), -ENODATA);
        expect("dropped_trusted_dot", get(D "/f", "trusted."), -ENODATA);
        expect("dropped_mode_0000_eacces", get(D "/none", "user.a"), -EACCES);
        expect("dropped_other_eacces", get(D "/other", "user.a"), -EACCES);
        expect("dropped_security_read", get(D "/none", "security.poly"), 3);
        expect("dropped_own_file", get(D "/f", "user.a"), -ENODATA);
    }
    /* Unlinking needs only write and search permission on D (root owns it). */
    a_unlink(D "/f");
    a_unlink(D "/none");
    a_unlink(D "/other");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
