/* flistxattr as root: with CAP_SYS_ADMIN trusted.* names are listed with
 * the user.* and security.* ones. After all capabilities are dropped, the
 * same descriptor lists the file without its trusted.* names (they are left
 * out of the list and out of the size-0 length; ERANGE is judged against
 * the shorter list), while user.* and security.* names are still listed.
 * poly-diff: needs=root (trusted.* and security.* need CAP_SYS_ADMIN)
 */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_fsetxattr 190
#define SYS_flistxattr 196
#define D "/tmp/a_flistxattr_root"

static char buf[256];
static i64 len;

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, z[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, z);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 flist(i64 fd) { return len = syscall3(SYS_flistxattr, fd, buf, sizeof buf); }

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
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    check("setup_open", fd >= 0, fd);
    expect("setup_user", syscall5(SYS_fsetxattr, fd, "user.u", "1", 1, 0), 0);
    expect("setup_trusted", syscall5(SYS_fsetxattr, fd, "trusted.t", "2", 1, 0), 0);
    expect("setup_trusted2", syscall5(SYS_fsetxattr, fd, "trusted.second", "", 0, 0), 0);
    expect("setup_security", syscall5(SYS_fsetxattr, fd, "security.poly", "3", 1, 0), 0);
    expect("all_length", flist(fd), 7 + 10 + 15 + 14);
    expect("all_user", has("user.u"), 1);
    expect("all_trusted", has("trusted.t"), 1);
    expect("all_trusted2", has("trusted.second"), 1);
    expect("all_security", has("security.poly"), 1);
    expect("all_size0", syscall3(SYS_flistxattr, fd, 0, 0), 46);
    expect("all_erange", syscall3(SYS_flistxattr, fd, buf, 45), -ERANGE);
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped == 0) {
        expect("dropped_length", flist(fd), 7 + 14);
        expect("dropped_no_trusted", has("trusted.t") + has("trusted.second"), 0);
        expect("dropped_user", has("user.u"), 1);
        expect("dropped_security", has("security.poly"), 1);
        expect("dropped_size0", syscall3(SYS_flistxattr, fd, 0, 0), 21);
        expect("dropped_exact", syscall3(SYS_flistxattr, fd, buf, 21), 21);
        expect("dropped_erange", syscall3(SYS_flistxattr, fd, buf, 20), -ERANGE);
    }
    a_close(fd);
    a_unlink(D "/f");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
