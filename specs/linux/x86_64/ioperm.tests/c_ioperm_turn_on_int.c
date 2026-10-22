/* turn_on is a 32-bit int: only the low 32 bits of the register count.
 * 2^32 means 0 (disable, succeeds without privilege); 2^32+1, 2, -1 and
 * 0x80000000 are non-zero (enable: EPERM without CAP_SYS_RAWIO).
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
    i64 r = ioperm(0x80, 1, 0x100000000L);
    check("turn_on_2_32_is_off", r == 0, r);
    r = ioperm(0x80, 1, (i64)0xffffffff00000000UL);
    check("turn_on_high_bits_only_is_off", r == 0, r);
    r = ioperm(0x80, 1, 0x100000001L);
    check("turn_on_2_32_plus_1_is_on", r == -EPERM, r);
    r = ioperm(0x80, 1, 2);
    check("turn_on_2_is_on", r == -EPERM, r);
    r = ioperm(0x80, 1, -1);
    check("turn_on_minus_1_is_on", r == -EPERM, r);
    r = ioperm(0x80, 1, 0x80000000L);
    check("turn_on_int_min_is_on", r == -EPERM, r);
    return finish();
}
