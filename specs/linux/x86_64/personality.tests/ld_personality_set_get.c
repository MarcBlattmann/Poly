/* personality(persona) returns the previous execution domain/persona and,
 * unless persona is 0xffffffff, replaces it. The persona is a 32-bit
 * unsigned value: the upper half of the register is ignored (so
 * 0xffffffffffffffff and 0x2ffffffff only read), and it is returned
 * zero-extended (0x80000000 and 0xfffffffe come back as positive values,
 * never as errors). Any 32-bit value is accepted, also undefined bits and
 * execution domains; nothing fails. The initial value is inherited from
 * the parent (a tracer or setarch may have set flags such as
 * ADDR_NO_RANDOMIZE), so this test only compares values it set itself and
 * puts the original back at the end.
 * poly-diff: syscalls=names (the first call returns the inherited persona)
 */
#include "a_sys.h"

#define SYS_personality 135
#define PER_LINUX 0x0000
#define PER_BSD 0x0006
#define ADDR_NO_RANDOMIZE 0x0040000
#define READ_IMPLIES_EXEC 0x0400000
#define QUERY 0xffffffffUL

static i64 pers(u64 v) { return syscall1(SYS_personality, v); }

int test_main(void) {
    i64 orig = pers(QUERY);
    check("query_nonnegative", orig >= 0 && orig <= 0xffffffffL, orig);
    check("query_again_same", pers(QUERY) == orig, 0);

    check("set_linux_returns_old", pers(PER_LINUX) == orig, 0);
    check("now_linux", pers(QUERY) == PER_LINUX, pers(QUERY));
    check("set_bsd_returns_linux", pers(PER_BSD) == PER_LINUX, 0);
    check("now_bsd", pers(QUERY) == PER_BSD, pers(QUERY));
    check("set_flags_returns_bsd", pers(ADDR_NO_RANDOMIZE | READ_IMPLIES_EXEC) == PER_BSD, 0);
    check("now_flags", pers(QUERY) == (ADDR_NO_RANDOMIZE | READ_IMPLIES_EXEC), pers(QUERY));

    check("set_arbitrary", pers(0x12345678) == (ADDR_NO_RANDOMIZE | READ_IMPLIES_EXEC), 0);
    check("now_arbitrary", pers(QUERY) == 0x12345678, pers(QUERY));
    check("set_top_bit", pers(0x80000000UL) == 0x12345678, 0);
    i64 v = pers(QUERY);
    check("top_bit_positive", v == 0x80000000L, v);
    check("set_fffffffe", pers(0xfffffffeUL) == 0x80000000L, 0);
    v = pers(QUERY);
    check("fffffffe_positive", v == 0xfffffffeL, v);

    check("set_high_bits_ignored", pers((1UL << 32) | 5) == 0xfffffffeL, 0);
    check("now_5", pers(QUERY) == 5, pers(QUERY));
    check("all_ones_64_is_query", pers(0xffffffffffffffffUL) == 5, 0);
    check("all_ones_64_changed_nothing", pers(QUERY) == 5, 0);
    check("2ffffffff_is_query", pers(0x2ffffffffUL) == 5, 0);
    check("still_5", pers(QUERY) == 5, 0);

    check("restore_returns_5", pers((u64)orig) == 5, 0);
    check("restored", pers(QUERY) == orig, 0);
    return finish();
}
