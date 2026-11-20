/* poly-mac: corefoundation */
/* The options act on the chosen range as on whole strings, and CFStringCompare
 * is the same call over the whole first string. */
#include "poly_mcf.h"

int test_main(void) {
    CFStringRef a = S("File10.txt"), b = S("file2.txt");
    CFRange all = CFRangeMake(0, CFStringGetLength(a));
    CKV(CFStringCompareWithOptions(a, b, all, 0) == -1, CFStringCompareWithOptions(a, b, all, 0));
    CKV(CFStringCompareWithOptions(a, b, all, kCFCompareCaseInsensitive) == -1, 0);
    CKV(CFStringCompareWithOptions(a, b, all, kCFCompareCaseInsensitive | kCFCompareNumerically) == 1, 0);
    CKV(CFStringCompareWithOptions(CFSTR("x5y"), CFSTR("5"), CFRangeMake(1, 1), kCFCompareNumerically) == 0, 0);
    CKV(CFStringCompareWithOptions(CFSTR("x50"), CFSTR("9"), CFRangeMake(1, 2), kCFCompareNumerically) == 1, 0);
    CFStringRef x = S("Mixed Case"), y = S("mixed case");
    CKV(CFStringCompareWithOptions(x, y, CFRangeMake(0, 10), kCFCompareCaseInsensitive) == CFStringCompare(x, y, kCFCompareCaseInsensitive), 0);
    CKV(CFStringCompareWithOptions(x, y, CFRangeMake(0, 10), 0) == CFStringCompare(x, y, 0), 0);
    CKV(CFStringCompareWithOptions(x, y, CFRangeMake(0, 5), kCFCompareCaseInsensitive) == -1, 0);
    return finish();
}
