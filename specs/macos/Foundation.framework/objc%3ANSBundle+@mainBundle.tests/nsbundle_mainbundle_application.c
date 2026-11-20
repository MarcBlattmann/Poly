/* poly-mac: foundation */
/* From inside an application (a copy of this test in T.app/Contents/MacOS/t):
 * the main bundle is that application, its path ends in T.app, it has the
 * identifier of the Info.plist and the same object every time; allBundles and
 * bundleWithIdentifier: find it; bundleWithPath: of its path gives the same bundle. */
#include "poly_mfn.h"

int test_main(void) {
    if (!cf_enter_app(CF_APP_PLIST)) return 1;
    NSBundle *b = [NSBundle mainBundle];
    CK(b != nil && b == [NSBundle mainBundle]);
    CK([b.bundlePath hasSuffix:@"/T.app"]);
    CK(ns_is(b.bundleIdentifier, "test.poly.probe"));
    CK([NSBundle bundleWithIdentifier:@"test.poly.probe"] == b);
    CK([NSBundle bundleWithIdentifier:@"test.poly.nobody"] == nil);
    CK([NSBundle bundleWithPath:b.bundlePath] == b);
    CK([b.executablePath hasSuffix:@"/T.app/Contents/MacOS/t"]);
    CK([b.resourcePath hasSuffix:@"/T.app/Contents/Resources"]);
    CK([[NSBundle allBundles] containsObject:b]);
    CK(b.bundleURL.isFileURL);
    return finish();
}
