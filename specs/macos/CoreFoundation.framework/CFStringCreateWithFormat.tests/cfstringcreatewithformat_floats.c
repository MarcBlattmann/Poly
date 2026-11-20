/* poly-mac: corefoundation */
/* The floating-point conversions follow printf: fixed, exponent, shortest;
 * precision, width, infinity and not-a-number. */
#pragma clang diagnostic ignored "-Wformat"
#include "poly_mcf.h"

#define F(...) CFStringCreateWithFormat(kCFAllocatorDefault, NULL, __VA_ARGS__)
#define IS(s, text) check(text, cf_is((s), text), 0)

int test_main(void) {
    IS(F(CFSTR("%f"), 3.14159), "3.141590");
    IS(F(CFSTR("%.2f"), 3.14159), "3.14");
    IS(F(CFSTR("[%10.3f]"), 3.14159), "[     3.142]");
    IS(F(CFSTR("%.0f"), 2.5), "2");
    IS(F(CFSTR("%.1f"), 0.05), "0.1");
    IS(F(CFSTR("%e"), 12345.678), "1.234568e+04");
    IS(F(CFSTR("%g"), 0.0001), "0.0001");
    IS(F(CFSTR("%g"), 1e10), "1e+10");
    IS(F(CFSTR("%f"), -0.0), "-0.000000");
    IS(F(CFSTR("%f"), __builtin_inf()), "inf");
    IS(F(CFSTR("%f"), __builtin_nan("")), "nan");
    IS(F(CFSTR("%lf"), 1.5), "1.500000");
    IS(F(CFSTR("%Lf"), (long double)1.5L), "1.500000");
    return finish();
}
