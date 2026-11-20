/* poly-mac: corefoundation */
/* The standard callbacks at work in a set: members are compared with CFEqual
 * and hashed with CFHash, so equal objects made apart are one member; numbers
 * of different types with the same value are one member; members are retained
 * and released. */
#include "poly_mcf.h"

int test_main(void) {
    CFMutableSetRef s = CFSetCreateMutable(kCFAllocatorDefault, 0, &kCFTypeSetCallBacks);
    CFSetAddValue(s, S("same"));
    CFSetAddValue(s, S("same"));
    CFSetAddValue(s, S("different"));
    CK(CFSetGetCount(s) == 2);
    int three = 3;
    long long three64 = 3;
    CFSetAddValue(s, CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &three));
    CFSetAddValue(s, CFNumberCreate(kCFAllocatorDefault, kCFNumberLongLongType, &three64));
    CK(CFSetGetCount(s) == 3);
    CK(CFSetContainsValue(s, S("different")) && !CFSetContainsValue(s, S("Different")));
    CFDataRef d = CFDataCreate(kCFAllocatorDefault, (const UInt8 *)"d", 1);
    CFSetAddValue(s, d);
    CKV(CFGetRetainCount(d) == 2, CFGetRetainCount(d));
    CFSetRemoveValue(s, d);
    CKV(CFGetRetainCount(d) == 1, CFGetRetainCount(d));
    return finish();
}
