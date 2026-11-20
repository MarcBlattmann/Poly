/* poly-mac: foundation */
/* In the application, +bundleWithIdentifier: finds the bundle by that string and not by a
 * different case or a prefix; CFBundleGetBundleWithIdentifier finds a bundle for it too (NSBundle and CFBundle are
 * separate objects, not bridged); the main bundle's bundleIdentifier and the dictionary's key
 * agree, and the identifier is a CFString. */
#include "poly_mfn.h"

int test_main(void) {
    if (!cf_enter_app(CF_APP_PLIST)) return 1;
    NSBundle *b = [NSBundle mainBundle];
    CK([NSBundle bundleWithIdentifier:@"test.poly.probe"] == b);
    CK([NSBundle bundleWithIdentifier:@"TEST.POLY.PROBE"] == nil);
    CK([NSBundle bundleWithIdentifier:@"test.poly"] == nil);
    CK([NSBundle bundleWithIdentifier:@""] == nil);
    CK(CFBundleGetBundleWithIdentifier(CFSTR("test.poly.probe")) != NULL);
    CK(CFGetTypeID((__bridge CFTypeRef)b.bundleIdentifier) == CFStringGetTypeID());
    CK([b.infoDictionary[@"CFBundleIdentifier"] isEqual:b.bundleIdentifier]);
    return finish();
}
