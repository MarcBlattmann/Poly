/* sysfs with an option other than 1, 2 or 3 fails with EINVAL, whatever
 * the other arguments are (NULL, unreadable or valid pointers). Only
 * the low 32 bits of the option count, and they are taken as a whole:
 * 0x80000003 is not 3, while 0x1_00000001 is 1.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sysfs 139

static i64 sysfs3(i64 opt, i64 a, i64 b) { return syscall3(SYS_sysfs, opt, a, b); }

int test_main(void) {
    char buf[256];
    i64 r = sysfs3(0, 0, 0);
    check("opt0_einval", r == -EINVAL, r);
    r = sysfs3(0, (i64)"proc", (i64)buf);
    check("opt0_valid_args_einval", r == -EINVAL, r);
    r = sysfs3(4, 0, 0);
    check("opt4_einval", r == -EINVAL, r);
    r = sysfs3(4, 1, 1);
    check("opt4_bad_ptrs_einval", r == -EINVAL, r);
    r = sysfs3(5, 0, (i64)buf);
    check("opt5_einval", r == -EINVAL, r);
    r = sysfs3(100, 0, 0);
    check("opt100_einval", r == -EINVAL, r);
    r = sysfs3(-1, 0, 0);
    check("opt_minus1_einval", r == -EINVAL, r);
    r = sysfs3(-3, 0, 0);
    check("opt_minus3_einval", r == -EINVAL, r);
    r = sysfs3(0x80000003L, 0, 0);
    check("opt_0x80000003_einval", r == -EINVAL, r);
    r = sysfs3(0x100000000L, 0, 0);
    check("opt_high_only_is_0_einval", r == -EINVAL, r);
    r = sysfs3(0x100000004L, 0, 0);
    check("opt_high_4_einval", r == -EINVAL, r);

    i64 p = sysfs3(1, (i64)"proc", 0);
    check("proc_known", p >= 0, p);
    r = sysfs3(1 | (1L << 32), (i64)"proc", 0);
    check("opt1_high_bits_ignored", r == p, r);
    memset(buf, 0, sizeof buf);
    r = sysfs3(2 | (0xffL << 48), p, (i64)buf);
    check("opt2_high_bits_ignored", r == 0, r);
    check("opt2_high_bits_name", str_eq(buf, "proc"), r);
    return finish();
}
