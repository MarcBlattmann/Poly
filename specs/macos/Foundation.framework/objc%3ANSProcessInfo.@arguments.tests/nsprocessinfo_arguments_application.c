/* poly-mac: foundation */
/* arguments is an array of strings, the program's argv: element 0 the path the program
 * was started with, the others exactly as given (here the test passes
 * '--in-app' and the argument 'two words'), UTF-8 decoded, in order; the array
 * is the same for repeated calls. */
#include "poly_mfn.h"

int test_main(void) {
    if (!cf_enter_app_ex(CF_APP_PLIST, "two words \xc3\xa9", NULL)) return 1;
    NSArray *args = [NSProcessInfo processInfo].arguments;
    CK(args != nil && args.count == 3);
    CK(ns_is(args[0], "T.app/Contents/MacOS/t") || [args[0] hasSuffix:@"/T.app/Contents/MacOS/t"]);
    CK(ns_is(args[1], "--in-app"));
    CK(ns_is(args[2], "two words \xc3\xa9"));
    for (id a in args) CK([a isKindOfClass:[NSString class]]);
    CK([[NSProcessInfo processInfo].arguments isEqualToArray:args]);
    return finish();
}
