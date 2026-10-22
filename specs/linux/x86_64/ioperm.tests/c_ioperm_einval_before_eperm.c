/* The range is checked before privilege: without capabilities, enabling
 * an invalid range (num 0, past port 65535, wrapping) is EINVAL, not EPERM;
 * the same valid range is EPERM.
 */
#include "poly_ipc.h"

#define SYS_ioperm 173
#define EPERM 1

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

static i64 ioperm(i64 from, i64 num, i64 on) { return syscall3(SYS_ioperm, from, num, on); }

int test_main(void) {
    struct cap_header h = { 0x20080522, 0 };
    struct cap_data d[2];
    memset(d, 0, sizeof d);
    check("capset_drop_all", syscall2(SYS_capset, &h, d) == 0, 0);
    i64 r = ioperm(0x80, 0, 1);
    check("num_0_on_einval", r == -EINVAL, r);
    r = ioperm(0, 65537, 1);
    check("too_long_on_einval", r == -EINVAL, r);
    r = ioperm(65536, 1, 1);
    check("from_65536_on_einval", r == -EINVAL, r);
    r = ioperm(1, -1, 1);
    check("wrapping_on_einval", r == -EINVAL, r);
    r = ioperm(0, 65536, 1);
    check("valid_on_eperm", r == -EPERM, r);
    return finish();
}
