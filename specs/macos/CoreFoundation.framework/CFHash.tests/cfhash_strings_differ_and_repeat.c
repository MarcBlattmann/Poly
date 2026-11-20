/* poly-mac: corefoundation */
/* The hash of a string is the same every time it is asked and, for these
 * different strings, different; the empty string hashes to 0. */
#include "poly_mcf.h"

int test_main(void) {
    CFStringRef a = S("alpha"), b = S("beta"), c = S("alphb"), e = S("");
    CK(CFHash(a) == CFHash(a));
    CK(CFHash(a) == CFHash(S("alpha")));
    CK(CFHash(a) != CFHash(b));
    CK(CFHash(a) != CFHash(c));
    CK(CFHash(b) != CFHash(c));
    CK(CFHash(e) == 0);
    CK(CFHash(e) == CFHash(CFSTR("")));
    CK(CFHash(S("abc")) != CFHash(S("cba")));
    return finish();
}
