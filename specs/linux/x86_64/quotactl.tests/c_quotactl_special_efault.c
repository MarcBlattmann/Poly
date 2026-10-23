/* A special that cannot be read is EFAULT (14): address 1, a kernel
 * address, a PROT_NONE page, and a name that runs into a PROT_NONE page
 * before its NUL. The same for Q_SYNC (special given), Q_GETFMT and an
 * unknown subcommand. A name ending just before the protected page is
 * read fine. */
#include "a_sys.h"

/* asm-generic/errno-base.h */
#define ENOTBLK 15

/* asm/unistd_64.h */
#define SYS_quotactl 179

/* linux/quota.h */
#define QCMD(cmd, type) ((((cmd) << 8) | ((type) & 0xff)))
#define Q_SYNC 0x800001
#define Q_GETFMT 0x800004

static i64 qc(i64 cmd, i64 special, i64 id, i64 addr) { return syscall4(SYS_quotactl, cmd, special, id, addr); }

int test_main(void) {
    char buf[64];
    char *m = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", (i64)m > 0, (i64)m);
    expect("mprotect", syscall3(SYS_mprotect, m + A_PAGE, A_PAGE, PROT_NONE), 0);
    char *tail = m + A_PAGE - 10;
    memset(tail, 'q', 10);
    tail[0] = '/';

    i64 cmds[3] = { QCMD(Q_SYNC, 0), QCMD(Q_GETFMT, 1), QCMD(0x1234, 2) };
    const char *names[3] = { "sync", "getfmt", "unknown" };
    for (int i = 0; i < 3; i++) {
        char n[64];
        str_cat2(n, names[i], "_addr_1");
        expect(n, qc(cmds[i], 1, 0, (i64)buf), -EFAULT);
        str_cat2(n, names[i], "_kernel");
        expect(n, qc(cmds[i], (i64)0xffff800000000000UL, 0, (i64)buf), -EFAULT);
        str_cat2(n, names[i], "_prot_none");
        expect(n, qc(cmds[i], (i64)(m + A_PAGE), 0, (i64)buf), -EFAULT);
        str_cat2(n, names[i], "_unterminated");
        expect(n, qc(cmds[i], (i64)tail, 0, (i64)buf), -EFAULT);
    }
    str_cpy(m + A_PAGE - 10, "/dev/null");
    expect("name_at_page_end", qc(QCMD(Q_GETFMT, 0), (i64)(m + A_PAGE - 10), 0, (i64)buf), -ENOTBLK);
    return finish();
}
