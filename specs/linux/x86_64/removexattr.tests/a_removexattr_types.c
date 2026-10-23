/* removexattr on files that are not regular files or directories, with
 * all capabilities dropped first: user.* attributes cannot exist there, so
 * user.* names (also "user.") on a FIFO or /dev/null are EPERM, even when
 * the FIFO is read-only; unknown prefixes are EOPNOTSUPP on a writable
 * FIFO or /dev/null and EACCES on a read-only FIFO; unknown system.* names
 * EOPNOTSUPP; trusted.* EPERM. A read-only /proc file gives EACCES for
 * user.* and EPERM for trusted.*. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_removexattr 197
#define SYS_mknodat 259
#define D "/tmp/a_removexattr_types"

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 rm(const char *p, const char *name) { return syscall2(SYS_removexattr, p, name); }

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", 0010644, 0), 0);
    expect("setup_rofifo", syscall4(SYS_mknodat, AT_FDCWD, D "/rofifo", 0010444, 0), 0);
    expect("fifo_user_eperm", rm(D "/fifo", "user.a"), -EPERM);
    expect("fifo_user_dot_eperm", rm(D "/fifo", "user."), -EPERM);
    expect("fifo_unknown_eopnotsupp", rm(D "/fifo", "foo.a"), -EOPNOTSUPP);
    expect("fifo_system_eopnotsupp", rm(D "/fifo", "system.x"), -EOPNOTSUPP);
    expect("fifo_trusted_eperm", rm(D "/fifo", "trusted.a"), -EPERM);
    expect("rofifo_user_eperm", rm(D "/rofifo", "user.a"), -EPERM);
    expect("rofifo_unknown_eacces", rm(D "/rofifo", "foo.a"), -EACCES);
    expect("devnull_user_eperm", rm("/dev/null", "user.a"), -EPERM);
    expect("devnull_unknown_eopnotsupp", rm("/dev/null", "foo.a"), -EOPNOTSUPP);
    expect("devnull_trusted_eperm", rm("/dev/null", "trusted.a"), -EPERM);
    expect("proc_user_eacces", rm("/proc/self/status", "user.a"), -EACCES);
    expect("proc_trusted_eperm", rm("/proc/self/status", "trusted.a"), -EPERM);
    expect("cleanup_fifo", a_unlink(D "/fifo"), 0);
    expect("cleanup_rofifo", a_unlink(D "/rofifo"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
