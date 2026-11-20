/* poly-mac: foundation */
/* NSDictionary and CFDictionary are one object: the CFDictionary functions work on
 * an NSDictionary (count, lookup, ContainsKey) and the NSDictionary methods on
 * a CFDictionary made by CFDictionaryCreate; the description has the form
 * '{\n    k = v;\n}'. */
#include "poly_mfn.h"

int test_main(void) {
    NSDictionary *d = @{@"k2": @"v2", @"k1": @"v1"};
    CFDictionaryRef cf = (__bridge CFDictionaryRef)d;
    CK(CFGetTypeID(cf) == CFDictionaryGetTypeID());
    CK(CFDictionaryGetCount(cf) == 2);
    CK(cf_is(CFDictionaryGetValue(cf, CFSTR("k2")), "v2"));
    CK(CFDictionaryContainsKey(cf, CFSTR("k1")) && !CFDictionaryContainsKey(cf, CFSTR("k3")));
    CFDictionaryRef made = CFDictionaryCreate(kCFAllocatorDefault, (const void *[]){CFSTR("a")}, (const void *[]){CFSTR("b")}, 1, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    NSDictionary *ns = (__bridge_transfer NSDictionary *)made;
    CK(ns.count == 1 && ns_is(ns[@"a"], "b"));
    CK([ns isEqualToDictionary:@{@"a": @"b"}]);
    CK(CFEqual((__bridge CFTypeRef)ns, (__bridge CFTypeRef)@{@"a": @"b"}));
    CK(ns_is([@{@"k": @"v"} description], "{\n    k = v;\n}"));
    NSMutableDictionary *m = (__bridge_transfer NSMutableDictionary *)CFDictionaryCreateMutable(kCFAllocatorDefault, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    m[@"x"] = @"1";
    CK(CFDictionaryGetCount((__bridge CFDictionaryRef)m) == 1);
    return finish();
}
