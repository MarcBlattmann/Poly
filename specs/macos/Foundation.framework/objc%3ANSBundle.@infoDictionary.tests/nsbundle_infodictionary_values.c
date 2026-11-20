/* poly-mac: foundation */
/* infoDictionary holds the Info.plist keys with property list types (string,
 * number, Boolean, array) plus the CFBundleNumericVersion that CoreFoundation
 * adds; objectForInfoDictionaryKey: reads one (nil for a missing or differently
 * cased key); a plain executable has an empty dictionary. */
#include "poly_mfn.h"

int test_main(void) {
    if (!cf_enter_app(CF_APP_PLIST)) return 1;
    NSBundle *b = [NSBundle mainBundle];
    NSDictionary *info = b.infoDictionary;
    CK(info != nil && info.count == 10);
    CK(ns_is(info[@"CFBundleName"], "Probe") && ns_is(info[@"CFBundleVersion"], "42"));
    CK(ns_is(info[@"CFBundleShortVersionString"], "1.2.3"));
    CK([info[@"MyInt"] isKindOfClass:[NSNumber class]] && [info[@"MyInt"] intValue] == 7);
    CK(info[@"MyBool"] == (id)kCFBooleanTrue && [info[@"MyBool"] boolValue]);
    CK([info[@"MyArr"] count] == 2 && ns_is(info[@"MyArr"][1], "b"));
    CK([info[@"CFBundleNumericVersion"] longLongValue] == 1107329024);
    CK(ns_is([b objectForInfoDictionaryKey:@"CFBundleName"], "Probe"));
    CK([b objectForInfoDictionaryKey:@"NoSuchKey"] == nil && [b objectForInfoDictionaryKey:@"cfbundlename"] == nil);
    CK(info == b.infoDictionary || [info isEqualToDictionary:b.infoDictionary]);
    CK(CFBundleGetInfoDictionary(CFBundleGetMainBundle()) != NULL);
    return finish();
}
