/* poly-mac: corefoundation */
/* Null, Booleans and numbers: the class name, address and allocator, then the
 * value in braces (addresses masked as 0xADDR). */
#include "poly_mcf.h"

static void d(const char *name, CFTypeRef o) {
    CFStringRef s = CFCopyDescription(o);
    value_masked(name, s);
    if (s) CFRelease(s);
}

int test_main(void) {
    d("null", kCFNull);
    d("true", kCFBooleanTrue);
    d("false", kCFBooleanFalse);
    int i = 1;
    long long big = 0x7fffffffffffffffLL;
    double dbl = 2.5;
    d("int", CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &i));
    d("long long", CFNumberCreate(kCFAllocatorDefault, kCFNumberLongLongType, &big));
    d("double", CFNumberCreate(kCFAllocatorDefault, kCFNumberDoubleType, &dbl));
    return finish();
}
