/* poly-mac: foundation */
/* stringWithFormat: formats like printf: integers of every size, bases, width,
 * precision, flags, floating point, %% and %c. */
#pragma clang diagnostic ignored "-Wformat"
#include "poly_mfn.h"

#define F(...) [NSString stringWithFormat:__VA_ARGS__]
#define IS(s, text) check(text, ns_is((s), text), 0)

int test_main(void) {
    IS(F(@"%d", -42), "-42");
    IS(F(@"%ld %lld %zu", -1L, -2LL, (size_t)3), "-1 -2 3");
    IS(F(@"%u", 4000000000u), "4000000000");
    IS(F(@"%x %X %o %#x", 255, 255, 8, 255), "ff FF 10 0xff");
    IS(F(@"[%5d][%-5d][%05d][%+d]", 42, 42, 42, 42), "[   42][42   ][00042][+42]");
    IS(F(@"[%*d]", 6, 42), "[    42]");
    IS(F(@"%f", 3.14159), "3.141590");
    IS(F(@"%.2f|%8.3f|%e|%g", 3.14159, 3.14159, 12345.678, 0.0001), "3.14|   3.142|1.234568e+04|0.0001");
    IS(F(@"%c%c", 'o', 'k'), "ok");
    IS(F(@"100%%"), "100%");
    IS(F(@"%d items", 3), "3 items");
    return finish();
}
