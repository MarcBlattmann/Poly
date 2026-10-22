/* The range [from, from+num) must lie in ports 0..65535: a range ending
 * exactly at 65536 is accepted, one byte more is EINVAL; a from of 65536
 * or above is EINVAL; from and num are full 64-bit values (a from of 2^32
 * is not truncated to 0).
 */
#include "poly_ipc.h"

#define SYS_ioperm 173

static i64 ioperm(i64 from, i64 num, i64 on) { return syscall3(SYS_ioperm, from, num, on); }

int test_main(void) {
    i64 r = ioperm(0, 65536, 0);
    check("end_65536_ok", r == 0, r);
    r = ioperm(65535, 1, 0);
    check("last_port_ok", r == 0, r);
    r = ioperm(0x8000, 0x8000, 0);
    check("upper_half_ok", r == 0, r);
    r = ioperm(0, 65537, 0);
    check("end_65537_einval", r == -EINVAL, r);
    r = ioperm(65535, 2, 0);
    check("last_port_plus_one_einval", r == -EINVAL, r);
    r = ioperm(65536, 1, 0);
    check("from_65536_einval", r == -EINVAL, r);
    r = ioperm(0x8000, 0x8001, 0);
    check("upper_half_plus_one_einval", r == -EINVAL, r);
    r = ioperm(0x100000000L, 1, 0);
    check("from_2_32_einval", r == -EINVAL, r);
    r = ioperm(0x80, 0x100000001L, 0);
    check("num_2_32_plus_1_einval", r == -EINVAL, r);
    return finish();
}
