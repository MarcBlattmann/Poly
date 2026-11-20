/* poly-mac: foundation */
/* A program started with no arguments has exactly one: its own path (an array of
 * one string whose last component is the executable's name); and the array and
 * every element are strings. */
#include "poly_mfn.h"

int test_main(void) {
    NSArray *args = [NSProcessInfo processInfo].arguments;
    CK(args != nil && args.count == 1);
    CK([args[0] isKindOfClass:[NSString class]]);
    CK(ns_is(args[0], poly_argv[0]));
    CK([[NSProcessInfo processInfo].arguments[0] length] > 0);
    return finish();
}
