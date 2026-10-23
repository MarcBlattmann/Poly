/* A NULL special is only meaningful for Q_SYNC: every other subcommand,
 * known (Q_GETFMT, Q_GETINFO, Q_GETQUOTA, Q_QUOTAON, the XFS Q_X* ones
 * including Q_XQUOTASYNC) or unknown, fails with ENODEV (19), whatever id
 * and addr are. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_quotactl 179

/* linux/quota.h */
#define QCMD(cmd, type) ((((cmd) << 8) | ((type) & 0xff)))
#define Q_QUOTAON 0x800002
#define Q_QUOTAOFF 0x800003
#define Q_GETFMT 0x800004
#define Q_GETINFO 0x800005
#define Q_SETINFO 0x800006
#define Q_GETQUOTA 0x800007
#define Q_SETQUOTA 0x800008
#define Q_GETNEXTQUOTA 0x800009
/* linux/dqblk_xfs.h: XQM_CMD(x) is ('X' << 8) + x */
#define Q_XGETQUOTA 0x5803
#define Q_XGETQSTAT 0x5805
#define Q_XQUOTASYNC 0x5807

static i64 qc(i64 cmd, i64 special, i64 id, i64 addr) { return syscall4(SYS_quotactl, cmd, special, id, addr); }

int test_main(void) {
    char buf[128];
    memset(buf, 0, sizeof buf);
    expect("quotaon", qc(QCMD(Q_QUOTAON, 0), 0, 2, (i64)"/tmp"), -ENODEV);
    expect("quotaon_bad_addr", qc(QCMD(Q_QUOTAON, 0), 0, 2, 1), -ENODEV);
    expect("quotaoff", qc(QCMD(Q_QUOTAOFF, 1), 0, 0, 0), -ENODEV);
    expect("getfmt", qc(QCMD(Q_GETFMT, 0), 0, 0, (i64)buf), -ENODEV);
    expect("getinfo", qc(QCMD(Q_GETINFO, 1), 0, 0, (i64)buf), -ENODEV);
    expect("setinfo", qc(QCMD(Q_SETINFO, 2), 0, 0, (i64)buf), -ENODEV);
    expect("getquota", qc(QCMD(Q_GETQUOTA, 0), 0, 0, (i64)buf), -ENODEV);
    expect("setquota", qc(QCMD(Q_SETQUOTA, 0), 0, 0, (i64)buf), -ENODEV);
    expect("getnextquota", qc(QCMD(Q_GETNEXTQUOTA, 1), 0, 0, (i64)buf), -ENODEV);
    expect("xgetquota", qc(QCMD(Q_XGETQUOTA, 0), 0, 0, (i64)buf), -ENODEV);
    expect("xgetqstat", qc(QCMD(Q_XGETQSTAT, 0), 0, 0, (i64)buf), -ENODEV);
    expect("xquotasync", qc(QCMD(Q_XQUOTASYNC, 0), 0, 0, 0), -ENODEV);
    expect("cmd_zero", qc(0, 0, 0, 0), -ENODEV);
    expect("unknown_cmd", qc(QCMD(0x1234, 2), 0, 0, 0), -ENODEV);
    expect("unknown_cmd_high_byte", qc(0x01800001, 0, 0, 0), -ENODEV);
    return finish();
}
