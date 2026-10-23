/* setxattr by file type: user.* attributes exist only on regular files and
 * directories; a FIFO, a character device (/dev/null) and a symbolic
 * link's target that is a FIFO give EPERM, before the write-permission
 * check and before "user." would be EINVAL. Other prefixes still get their
 * own answers on a FIFO: unknown ones EOPNOTSUPP (EACCES first when the
 * FIFO is not writable), unknown system.* EOPNOTSUPP. Runs with all
 * capabilities dropped, so root is checked like anyone else. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_mknodat 259
#define D "/tmp/a_setxattr_types"

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 set(const char *p, const char *name) { return syscall5(SYS_setxattr, p, name, "v", 1, 0); }

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", S_IFIFO | 0644, 0), 0);
    expect("setup_rofifo", syscall4(SYS_mknodat, AT_FDCWD, D "/rofifo", S_IFIFO | 0444, 0), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "fifo", AT_FDCWD, D "/lfifo"), 0);
    expect("fifo_user_eperm", set(D "/fifo", "user.a"), -EPERM);
    expect("fifo_user_dot_eperm", set(D "/fifo", "user."), -EPERM);
    expect("ro_fifo_user_eperm", set(D "/rofifo", "user.a"), -EPERM);
    expect("fifo_unknown_ns_eopnotsupp", set(D "/fifo", "foo.a"), -EOPNOTSUPP);
    expect("ro_fifo_unknown_ns_eacces", set(D "/rofifo", "foo.a"), -EACCES);
    expect("fifo_system_eopnotsupp", set(D "/fifo", "system.a"), -EOPNOTSUPP);
    expect("link_to_fifo_eperm", set(D "/lfifo", "user.a"), -EPERM);
    expect("dev_null_user_eperm", set("/dev/null", "user.a"), -EPERM);
    expect("dev_null_user_dot_eperm", set("/dev/null", "user."), -EPERM);
    expect("cleanup_link", a_unlink(D "/lfifo"), 0);
    expect("cleanup_fifo", a_unlink(D "/fifo"), 0);
    expect("cleanup_rofifo", a_unlink(D "/rofifo"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
