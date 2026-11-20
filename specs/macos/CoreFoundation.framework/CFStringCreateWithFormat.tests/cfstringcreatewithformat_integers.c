/* poly-mac: corefoundation */
/* The integer conversions of printf: sizes, signs, bases, width, precision,
 * flags and the star width. */
#pragma clang diagnostic ignored "-Wformat"
#include "poly_mcf.h"

#define F(...) CFStringCreateWithFormat(kCFAllocatorDefault, NULL, __VA_ARGS__)
#define IS(s, text) check(text, cf_is((s), text), 0)

int test_main(void) {
    IS(F(CFSTR("%d"), -42), "-42");
    IS(F(CFSTR("%i"), 12), "12");
    IS(F(CFSTR("%ld"), -4000000000L), "-4000000000");
    IS(F(CFSTR("%lld"), -4000000000LL), "-4000000000");
    IS(F(CFSTR("%u"), 4000000000u), "4000000000");
    IS(F(CFSTR("%lu"), 18446744073709551615UL), "18446744073709551615");
    IS(F(CFSTR("%zu"), (size_t)7), "7");
    IS(F(CFSTR("%hd"), (short)-3), "-3");
    IS(F(CFSTR("%hhd"), (signed char)-3), "-3");
    IS(F(CFSTR("%hu"), (unsigned short)65535), "65535");
    IS(F(CFSTR("%x"), 255), "ff");
    IS(F(CFSTR("%X"), 255), "FF");
    IS(F(CFSTR("%#x"), 255), "0xff");
    IS(F(CFSTR("%08x"), 0xbeef), "0000beef");
    IS(F(CFSTR("%llx"), 0x1234567890abcdefULL), "1234567890abcdef");
    IS(F(CFSTR("%o"), 8), "10");
    IS(F(CFSTR("[%5d]"), 42), "[   42]");
    IS(F(CFSTR("[%-5d]"), 42), "[42   ]");
    IS(F(CFSTR("[%05d]"), 42), "[00042]");
    IS(F(CFSTR("[%+d]"), 42), "[+42]");
    IS(F(CFSTR("[% d]"), 42), "[ 42]");
    IS(F(CFSTR("[%.3d]"), 7), "[007]");
    IS(F(CFSTR("[%*d]"), 6, 42), "[    42]");
    IS(F(CFSTR("%td"), (long)5), "5");
    return finish();
}
