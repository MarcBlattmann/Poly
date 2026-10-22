/* personality: only the low 32 bits of the argument count. A value
 * whose low 32 bits are 0xffffffff is a query, whatever the upper half;
 * otherwise the upper half is dropped. The result is the old persona as
 * an unsigned 32-bit number (bit 31 set is a large positive result, not
 * an error). */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_personality 135
#define QUERY 0xffffffffUL

static i64 personality(u64 p) { return syscall1(SYS_personality, p); }

int test_main(void) {
    i64 initial = personality(QUERY);
    expect("set_with_high_bits", personality(0x100000006UL), initial);
    expect("high_bits_dropped", personality(QUERY), 6);
    expect("all_ones_is_query", personality(~0UL), 6);
    expect("all_ones_changed_nothing", personality(QUERY), 6);
    expect("high_bit_query", personality(0x1ffffffffUL), 6);
    expect("high_bit_query_changed_nothing", personality(QUERY), 6);
    expect("high_half_only_sets_zero", personality(0xffffffff00000000UL), 6);
    expect("now_zero", personality(QUERY), 0);
    expect("set_bit31", personality(0x80000000UL), 0);
    expect("bit31_not_sign_extended", personality(QUERY), 0x80000000L);
    expect("set_fffffffe_returns_bit31", personality(0xfffffffeUL), 0x80000000L);
    expect("fffffffe_positive", personality(QUERY), 0xfffffffeL);
    expect("set_7fffffff", personality(0x7fffffffUL), 0xfffffffeL);
    expect("query_7fffffff", personality(QUERY), 0x7fffffffL);
    expect("reset", personality(0x5a00000000000000UL), 0x7fffffffL);
    expect("reset_zero", personality(QUERY), 0);
    return finish();
}
