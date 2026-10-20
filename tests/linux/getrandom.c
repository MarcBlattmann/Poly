/* getrandom: fills the buffer, two calls differ, bad flags fail. */
#include "poly_test.h"

int test_main(void) {
    unsigned char a[16], b[16];
    memset(a, 0, sizeof a);
    memset(b, 0, sizeof b);
    i64 r = syscall3(SYS_getrandom, a, sizeof a, 0);
    check("getrandom_fills_buffer", r == 16, r);
    r = syscall3(SYS_getrandom, b, sizeof b, GRND_NONBLOCK);
    check("getrandom_nonblock_fills_buffer", r == 16, r);
    int differ = 0;
    for (int i = 0; i < 16; i++) differ |= a[i] != b[i];
    check("getrandom_calls_differ", differ, 0);
    r = syscall3(SYS_getrandom, a, 0, 0);
    check("getrandom_zero_length", r == 0, r);
    r = syscall3(SYS_getrandom, a, sizeof a, 0x40000000);
    check("getrandom_bad_flags_einval", r == -EINVAL, r);
    r = syscall3(SYS_getrandom, 8, 16, 0);
    check("getrandom_bad_pointer_efault", r == -EFAULT, r);
    return finish();
}
