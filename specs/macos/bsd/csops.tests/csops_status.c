/* csops, BSD 169 (pid, operation, buffer, size): CS_OPS_STATUS (0) stores
 * the code-signing flags (u32) of the process; pid 0 is the caller; the
 * flags of a process run from a signed executable have CS_VALID (1). The flags
 * themselves are a value (ad-hoc signed by the linker on this machine). */
#include "poly_mtest.h"

/* Operations and flags: no public header; found by experiment. */
#define CS_OPS_STATUS 0
#define CS_VALID 0x00000001u

int test_main(void) {
    u32 a = 0xffffffff, b = 0xffffffff;
    sysret r = bsd4(SYS_csops, 0, CS_OPS_STATUS, &a, 4);
    check("pid 0 (the caller): returns 0", r.err == 0 && r.ret == 0, r.err);
    r = bsd4(SYS_csops, bsd0(SYS_getpid).ret, CS_OPS_STATUS, &b, 4);
    check("the caller's own pid: the same flags", r.err == 0 && a == b, b);
    check("CS_VALID is set", (a & CS_VALID) != 0, a);
    value("code-signing flags", a);
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        u32 c = 0;
        bsd4(SYS_csops, 0, CS_OPS_STATUS, &c, 4);
        poly_exit(c == a ? 0 : 1);
    }
    int st = poly_wait(f.ret);
    check("a forked child has the same flags", W_EXITED(st) && W_EXITSTATUS(st) == 0, st);
    return finish();
}
