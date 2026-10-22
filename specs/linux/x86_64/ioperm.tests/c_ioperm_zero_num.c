/* ioperm with num 0 is EINVAL, whatever from and turn_on are (an empty
 * range is not accepted, not even to disable).
 */
#include "poly_ipc.h"

#define SYS_ioperm 173

static i64 ioperm(i64 from, i64 num, i64 on) { return syscall3(SYS_ioperm, from, num, on); }

int test_main(void) {
    i64 r = ioperm(0, 0, 0);
    check("from_0_num_0_off", r == -EINVAL, r);
    r = ioperm(0x80, 0, 0);
    check("from_0x80_num_0_off", r == -EINVAL, r);
    r = ioperm(65535, 0, 0);
    check("from_65535_num_0_off", r == -EINVAL, r);
    r = ioperm(65536, 0, 0);
    check("from_65536_num_0_off", r == -EINVAL, r);
    r = ioperm(0, 0, 1);
    check("from_0_num_0_on", r == -EINVAL, r);
    r = ioperm(0x80, 0, 1);
    check("from_0x80_num_0_on", r == -EINVAL, r);
    return finish();
}
