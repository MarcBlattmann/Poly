/* poly-mac: corefoundation */
/* Positional arguments (%2$@), an unknown conversion (its letter is written
 * as it is, no argument used), a lone trailing percent sign (dropped) and %p. */
#pragma clang diagnostic ignored "-Wformat"
#pragma clang diagnostic ignored "-Wformat-invalid-specifier"
#pragma clang diagnostic ignored "-Wformat-extra-args"
#include "poly_mcf.h"

#define F(...) CFStringCreateWithFormat(kCFAllocatorDefault, NULL, __VA_ARGS__)
#define IS(s, text) check(text, cf_is((s), text), 0)

int test_main(void) {
    IS(F(CFSTR("%2$@ %1$@"), CFSTR("one"), CFSTR("two")), "two one");
    IS(F(CFSTR("%2$d-%1$d"), 1, 2), "2-1");
    IS(F(CFSTR("%3$d"), 1, 2, 3), "3");
    IS(F(CFSTR("%p"), (void *)0), "0x0");
    IS(F(CFSTR("%y")), "y");
    IS(F(CFSTR("abc%")), "abc");
    return finish();
}
