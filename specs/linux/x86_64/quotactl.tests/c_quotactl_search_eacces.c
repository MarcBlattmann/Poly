/* Resolving the special needs search permission on its directories: with
 * all capabilities dropped (so root and a normal user behave alike), a
 * name under a mode 000 directory the caller owns is EACCES (13), for an
 * existing and for a missing entry; once the mode is back, the same names
 * give ENOTBLK and ENOENT. */
#include "a_sys.h"

/* asm-generic/errno-base.h */
#define ENOTBLK 15

/* asm/unistd_64.h */
#define SYS_quotactl 179
#define SYS_capset 126

/* linux/quota.h */
#define QCMD(cmd, type) ((((cmd) << 8) | ((type) & 0xff)))
#define Q_SYNC 0x800001
#define Q_GETFMT 0x800004

#define D "/tmp/c_quotactl_search_eacces"

struct cap_header {
    u32 version;
    i32 pid;
};
struct cap_data {
    u32 effective, permitted, inheritable;
};

static i64 drop_caps(void) {
    struct cap_header h = { 0x20080522, 0 }; /* _LINUX_CAPABILITY_VERSION_3 */
    struct cap_data d[2];
    memset(d, 0, sizeof d);
    return syscall2(SYS_capset, &h, d);
}

static i64 qc(i64 cmd, const char *special) {
    char buf[16];
    return syscall4(SYS_quotactl, cmd, special, 0, buf);
}

int test_main(void) {
    a_unlink(D "/file");
    a_rmdir(D);
    expect("mkdir", a_mkdir(D, 0755), 0);
    expect("make_file", a_make_file(D "/file", "x", 0644), 0);
    expect("drop_caps", drop_caps(), 0);
    expect("chmod_000", syscall2(SYS_chmod, D, 0), 0);

    expect("existing_eacces", qc(QCMD(Q_GETFMT, 0), D "/file"), -EACCES);
    expect("missing_eacces", qc(QCMD(Q_GETFMT, 0), D "/none"), -EACCES);
    expect("sync_eacces", qc(QCMD(Q_SYNC, 1), D "/file"), -EACCES);
    expect("dir_itself_notblk", qc(QCMD(Q_GETFMT, 0), D), -ENOTBLK);

    expect("chmod_755", syscall2(SYS_chmod, D, 0755), 0);
    expect("existing_notblk", qc(QCMD(Q_GETFMT, 0), D "/file"), -ENOTBLK);
    expect("missing_enoent", qc(QCMD(Q_GETFMT, 0), D "/none"), -ENOENT);

    a_unlink(D "/file");
    expect("cleanup", a_rmdir(D), 0);
    return finish();
}
