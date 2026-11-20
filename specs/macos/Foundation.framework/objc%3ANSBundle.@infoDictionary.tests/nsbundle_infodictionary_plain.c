/* poly-mac: foundation */
/* Outside a bundle the dictionary is empty and non-nil, every key gives nil, and
 * the Foundation view equals the CoreFoundation view (empty too). */
#include "poly_mfn.h"

int test_main(void) {
    NSBundle *b = [NSBundle mainBundle];
    NSDictionary *info = b.infoDictionary;
    CK(info != nil && info.count == 0);
    CK([b objectForInfoDictionaryKey:@"CFBundleIdentifier"] == nil);
    CK([b objectForInfoDictionaryKey:@"CFBundleName"] == nil);
    CFDictionaryRef cf = CFBundleGetInfoDictionary(CFBundleGetMainBundle());
    CK(cf != NULL && CFDictionaryGetCount(cf) == 0);
    CK(CFGetTypeID((__bridge CFTypeRef)info) == CFDictionaryGetTypeID());
    return finish();
}
