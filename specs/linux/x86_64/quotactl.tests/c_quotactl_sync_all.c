/* Q_SYNC with a NULL special syncs every file system that has quotas on
 * and returns 0, for each of the three types, without any privilege (all
 * capabilities dropped first). id and addr are ignored then (junk values
 * and unreadable addresses are fine), and only the low 32 bits of cmd
 * count. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_quotactl 179
#define SYS_capset 126

/* linux/quota.h */
#define QCMD(cmd, type) ((((cmd) << 8) | ((type) & 0xff)))
#define Q_SYNC 0x800001

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

static i64 qc(i64 cmd, i64 special, i64 id, i64 addr) { return syscall4(SYS_quotactl, cmd, special, id, addr); }

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("sync_usr", qc(QCMD(Q_SYNC, 0), 0, 0, 0), 0);
    expect("sync_grp", qc(QCMD(Q_SYNC, 1), 0, 0, 0), 0);
    expect("sync_prj", qc(QCMD(Q_SYNC, 2), 0, 0, 0), 0);
    expect("sync_junk_id", qc(QCMD(Q_SYNC, 0), 0, -1, 0), 0);
    expect("sync_bad_addr", qc(QCMD(Q_SYNC, 1), 0, 12345, 1), 0);
    expect("sync_kernel_addr", qc(QCMD(Q_SYNC, 2), 0, 0, (i64)0xffff800000000000UL), 0);
    expect("sync_again", qc(QCMD(Q_SYNC, 0), 0, 0, 0), 0);
    /* cmd is a 32-bit value: QCMD(Q_SYNC, 0) is 0x80000100, negative as
     * an int; sign-extended or with junk in bits 32-63 it is the same. */
    expect("sync_sign_extended", qc((i64)0xffffffff80000100UL, 0, 0, 0), 0);
    expect("sync_high_junk", qc((i64)0x1234567880000100UL, 0, 0, 0), 0);
    expect("sync_high_junk_type3", qc((i64)0x1234567880000103UL, 0, 0, 0), -EINVAL);
    return finish();
}
