/* Q_QUOTAON takes a quota file path name in addr, but problems with that
 * path are never reported before those of the special: with an
 * unreadable or missing addr, a bad type is still EINVAL, a NULL special
 * ENODEV, a missing special ENOENT and a character device ENOTBLK. The
 * other subcommands do not read addr at that stage either. */
#include "a_sys.h"

/* asm-generic/errno-base.h */
#define ENOTBLK 15

/* asm/unistd_64.h */
#define SYS_quotactl 179

/* linux/quota.h */
#define QCMD(cmd, type) ((((cmd) << 8) | ((type) & 0xff)))
#define Q_QUOTAON 0x800002
#define Q_QUOTAOFF 0x800003
#define Q_GETFMT 0x800004
#define Q_SETQUOTA 0x800008
#define QFMT_VFS_V1 4

static i64 qc(i64 cmd, i64 special, i64 id, i64 addr) { return syscall4(SYS_quotactl, cmd, special, id, addr); }

int test_main(void) {
    const char *missing = "/tmp/c_quotactl_quotaon_addr_none";
    i64 on = QCMD(Q_QUOTAON, 0);
    expect("bad_type", qc(QCMD(Q_QUOTAON, 3), (i64)"/dev/null", QFMT_VFS_V1, 1), -EINVAL);
    expect("null_special_efault_addr", qc(on, 0, QFMT_VFS_V1, 1), -ENODEV);
    expect("null_special_missing_addr", qc(on, 0, QFMT_VFS_V1, (i64)missing), -ENODEV);
    expect("missing_special_efault_addr", qc(on, (i64)missing, QFMT_VFS_V1, 1), -ENOENT);
    expect("efault_special_missing_addr", qc(on, 1, QFMT_VFS_V1, (i64)"/tmp/c_quotactl_x/y"), -EFAULT);
    expect("chardev_efault_addr", qc(on, (i64)"/dev/null", QFMT_VFS_V1, 1), -ENOTBLK);
    expect("chardev_missing_addr", qc(on, (i64)"/dev/null", QFMT_VFS_V1, (i64)missing), -ENOTBLK);
    expect("chardev_null_addr", qc(on, (i64)"/dev/null", QFMT_VFS_V1, 0), -ENOTBLK);
    expect("chardev_any_format", qc(on, (i64)"/dev/null", 999, (i64)"/tmp"), -ENOTBLK);
    expect("quotaoff_chardev_efault_addr", qc(QCMD(Q_QUOTAOFF, 1), (i64)"/dev/null", 0, 1), -ENOTBLK);
    expect("setquota_chardev_efault_addr", qc(QCMD(Q_SETQUOTA, 0), (i64)"/dev/null", 0, 1), -ENOTBLK);
    expect("getfmt_chardev_efault_addr", qc(QCMD(Q_GETFMT, 2), (i64)"/dev/null", 0, 1), -ENOTBLK);
    return finish();
}
