/* A special that does not name anything is ENOENT (2): a missing file, a
 * missing directory on the way, an empty string and a dangling symbolic
 * link. Unknown subcommands and Q_SYNC with a special get the same. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_quotactl 179

/* linux/quota.h */
#define QCMD(cmd, type) ((((cmd) << 8) | ((type) & 0xff)))
#define Q_SYNC 0x800001
#define Q_GETFMT 0x800004
#define Q_GETQUOTA 0x800007

#define D "/tmp/c_quotactl_special_enoent"

static i64 qc(i64 cmd, const char *special, i64 id, void *addr) {
    return syscall4(SYS_quotactl, cmd, special, id, addr);
}

int test_main(void) {
    char buf[128];
    a_unlink(D "/dangling");
    a_rmdir(D);
    expect("mkdir", a_mkdir(D, 0755), 0);
    expect("symlink", syscall2(SYS_symlink, D "/nowhere", D "/dangling"), 0);

    expect("missing_file", qc(QCMD(Q_GETFMT, 0), D "/none", 0, buf), -ENOENT);
    expect("missing_dir", qc(QCMD(Q_GETFMT, 0), D "/none/dev", 0, buf), -ENOENT);
    expect("empty", qc(QCMD(Q_GETFMT, 0), "", 0, buf), -ENOENT);
    expect("dangling_symlink", qc(QCMD(Q_GETFMT, 1), D "/dangling", 0, buf), -ENOENT);
    expect("sync_missing", qc(QCMD(Q_SYNC, 0), D "/none", 0, 0), -ENOENT);
    expect("getquota_missing", qc(QCMD(Q_GETQUOTA, 0), D "/none", 0, buf), -ENOENT);
    expect("unknown_cmd_missing", qc(QCMD(0x1234, 0), D "/none", 0, buf), -ENOENT);
    expect("cmd_zero_missing", qc(0, D "/none", 0, buf), -ENOENT);

    expect("cleanup_link", a_unlink(D "/dangling"), 0);
    expect("cleanup_dir", a_rmdir(D), 0);
    return finish();
}
