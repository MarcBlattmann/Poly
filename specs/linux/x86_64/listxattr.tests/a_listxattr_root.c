/* listxattr as root: with CAP_SYS_ADMIN trusted.* names are listed along
 * with user.* and security.* ones; root may also set security.* names.
 * After all capabilities are dropped, the trusted.* names are left out of
 * the list (and out of the size-0 length), while user.* and security.*
 * names are still listed. A symbolic link may hold trusted.* attributes,
 * which llistxattr lists only with CAP_SYS_ADMIN.
 * poly-diff: needs=root (trusted.* and security.* need CAP_SYS_ADMIN)
 */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_lsetxattr 189
#define SYS_listxattr 194
#define SYS_llistxattr 195
#define D "/tmp/a_listxattr_root"

static char buf[256];
static i64 len;

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 list(const char *p) { return len = syscall3(SYS_listxattr, p, buf, sizeof buf); }
static i64 llist(const char *p) { return len = syscall3(SYS_llistxattr, p, buf, sizeof buf); }

static int has(const char *name) {
    int n = 0;
    for (i64 i = 0; i < len; i += str_len(buf + i) + 1)
        if (str_len(buf + i) == str_len(name) && mem_eq(buf + i, name, str_len(name))) n++;
    return n;
}

int test_main(void) {
    i64 uid = syscall0(SYS_geteuid);
    expect("is_root", uid, 0);
    if (uid != 0) return finish();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_link", syscall2(SYS_symlink, "f", D "/ln"), 0);
    expect("setup_user", syscall5(SYS_setxattr, D "/f", "user.u", "1", 1, 0), 0);
    expect("setup_trusted", syscall5(SYS_setxattr, D "/f", "trusted.t", "2", 1, 0), 0);
    expect("setup_security", syscall5(SYS_setxattr, D "/f", "security.poly", "3", 1, 0), 0);
    expect("setup_link_trusted", syscall5(SYS_lsetxattr, D "/ln", "trusted.l", "4", 1, 0), 0);
    expect("all_length", list(D "/f"), 7 + 10 + 14);
    expect("all_user", has("user.u"), 1);
    expect("all_trusted", has("trusted.t"), 1);
    expect("all_security", has("security.poly"), 1);
    expect("all_size0", syscall3(SYS_listxattr, D "/f", 0, 0), 31);
    expect("link_trusted", llist(D "/ln"), 10);
    expect("link_trusted_name", has("trusted.l"), 1);
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped == 0) {
        expect("dropped_length", list(D "/f"), 7 + 14);
        expect("dropped_no_trusted", has("trusted.t"), 0);
        expect("dropped_user", has("user.u"), 1);
        expect("dropped_security", has("security.poly"), 1);
        expect("dropped_size0", syscall3(SYS_listxattr, D "/f", 0, 0), 21);
        expect("dropped_erange", syscall3(SYS_listxattr, D "/f", buf, 20), -34);
        expect("dropped_link_empty", llist(D "/ln"), 0);
    }
    a_unlink(D "/ln");
    a_unlink(D "/f");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
