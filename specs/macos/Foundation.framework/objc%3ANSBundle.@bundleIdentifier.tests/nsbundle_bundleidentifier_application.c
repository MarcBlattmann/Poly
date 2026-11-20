/* poly-mac: foundation */
/* bundleIdentifier is the CFBundleIdentifier string of the Info.plist, the same
 * string as CFBundleGetIdentifier and as objectForInfoDictionaryKey:; bundles
 * made from a directory without Info.plist have none. */
#include "poly_mfn.h"

int test_main(void) {
    if (!cf_enter_app(CF_APP_PLIST)) return 1;
    NSBundle *b = [NSBundle mainBundle];
    NSString *id_ = b.bundleIdentifier;
    CK(id_ != nil && ns_is(id_, "test.poly.probe"));
    CK(CFBundleGetIdentifier(CFBundleGetMainBundle()) == (__bridge CFStringRef)id_);
    CK([[b objectForInfoDictionaryKey:@"CFBundleIdentifier"] isEqual:id_]);
    NSBundle *res = [NSBundle bundleWithPath:[b.bundlePath stringByAppendingPathComponent:@"Contents/Resources"]];
    CK(res != nil && res.bundleIdentifier == nil);
    CK([NSBundle bundleWithPath:@"/nonexistent-poly-dir"] == nil);
    return finish();
}
