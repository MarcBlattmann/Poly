/* A special that exists but is not a block device is ENOTBLK (15): a
 * character device, a directory, a regular file (even mode 000: it is not
 * opened), a FIFO, a symbolic link to /dev/null (links are followed) and
 * a relative name, resolved from the current directory. This is reported
 * before the subcommand is looked at, so unknown subcommands and Q_SYNC
 * get it too. */
#include "a_sys.h"

/* asm-generic/errno-base.h */
#define ENOTBLK 15

/* asm/unistd_64.h */
#define SYS_quotactl 179
#define SYS_mknodat 259

/* linux/quota.h */
#define QCMD(cmd, type) ((((cmd) << 8) | ((type) & 0xff)))
#define Q_SYNC 0x800001
#define Q_QUOTAON 0x800002
#define Q_GETFMT 0x800004
#define Q_GETQUOTA 0x800007

#define D "/tmp/c_quotactl_notblk"

static i64 qc(i64 cmd, const char *special, i64 id, void *addr) {
    return syscall4(SYS_quotactl, cmd, special, id, addr);
}

int test_main(void) {
    char buf[128];
    a_unlink(D "/file");
    a_unlink(D "/fifo");
    a_unlink(D "/null");
    a_rmdir(D);
    expect("mkdir", a_mkdir(D, 0755), 0);
    expect("make_file", a_make_file(D "/file", "x", 0), 0);
    expect("mkfifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", S_IFIFO | 0644, 0), 0);
    expect("symlink", syscall2(SYS_symlink, "/dev/null", D "/null"), 0);

    expect("chardev", qc(QCMD(Q_GETFMT, 0), "/dev/null", 0, buf), -ENOTBLK);
    expect("directory", qc(QCMD(Q_GETFMT, 0), D, 0, buf), -ENOTBLK);
    expect("root_dir", qc(QCMD(Q_GETFMT, 1), "/", 0, buf), -ENOTBLK);
    expect("regular_mode_000", qc(QCMD(Q_GETFMT, 2), D "/file", 0, buf), -ENOTBLK);
    expect("fifo", qc(QCMD(Q_GETFMT, 0), D "/fifo", 0, buf), -ENOTBLK);
    expect("symlink_to_chardev", qc(QCMD(Q_GETFMT, 0), D "/null", 0, buf), -ENOTBLK);
    expect("relative_from_root", qc(QCMD(Q_GETFMT, 0), "dev/null", 0, buf), -ENOTBLK);
    expect("sync_chardev", qc(QCMD(Q_SYNC, 0), "/dev/null", 0, 0), -ENOTBLK);
    expect("getquota_chardev", qc(QCMD(Q_GETQUOTA, 0), "/dev/null", 0, buf), -ENOTBLK);
    expect("quotaon_chardev", qc(QCMD(Q_QUOTAON, 0), "/dev/null", 2, (void *)"/tmp"), -ENOTBLK);
    expect("unknown_cmd_chardev", qc(QCMD(0x1234, 0), "/dev/null", 0, buf), -ENOTBLK);
    expect("cmd_zero_dir", qc(0, D, 0, buf), -ENOTBLK);

    a_unlink(D "/file");
    a_unlink(D "/fifo");
    a_unlink(D "/null");
    expect("cleanup", a_rmdir(D), 0);
    return finish();
}
