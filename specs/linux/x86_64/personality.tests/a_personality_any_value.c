/* personality: every value other than the query value is accepted and
 * stored as given, on x86-64: all 256 execution domains in the low
 * byte, every single bit, and a run of mixed values. No call fails. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_personality 135
#define QUERY 0xffffffffUL

static i64 personality(u64 p) { return syscall1(SYS_personality, p); }

int test_main(void) {
    u64 prev = (u64)personality(QUERY);
    int bad_ret = 0, bad_get = 0;
    for (u64 v = 0; v < 256; v++) {
        if ((u64)personality(v) != prev) bad_ret++;
        if ((u64)personality(QUERY) != v) bad_get++;
        prev = v;
    }
    expect("low_byte_returns_old", bad_ret, 0);
    expect("low_byte_stored", bad_get, 0);
    bad_ret = bad_get = 0;
    for (int b = 0; b < 32; b++) {
        u64 v = 1UL << b;
        if ((u64)personality(v) != prev) bad_ret++;
        if ((u64)personality(QUERY) != v) bad_get++;
        prev = v;
    }
    expect("single_bits_return_old", bad_ret, 0);
    expect("single_bits_stored", bad_get, 0);
    bad_ret = bad_get = 0;
    u64 x = 0x243f6a8885a308d3UL;
    for (int i = 0; i < 4096; i++) {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        u64 v = x & 0xffffffffUL;
        if (v == QUERY) continue;
        if ((u64)personality(v) != prev) bad_ret++;
        if ((u64)personality(QUERY) != v) bad_get++;
        prev = v;
    }
    expect("mixed_return_old", bad_ret, 0);
    expect("mixed_stored", bad_get, 0);
    expect("all_but_one_bit", personality(0xfffffffeUL), (i64)prev);
    expect("all_but_one_bit_stored", personality(QUERY), 0xfffffffeL);
    expect("reset", personality(0), 0xfffffffeL);
    return finish();
}
