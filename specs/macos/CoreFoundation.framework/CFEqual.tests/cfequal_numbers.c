/* poly-mac: corefoundation */
/* Numbers compare by value, not by the type they were made with. */
#include "poly_mcf.h"

static CFNumberRef mk(CFNumberType t, const void *p) { return CFNumberCreate(kCFAllocatorDefault, t, p); }

int test_main(void) {
    int i1 = 1;
    long long l1 = 1;
    short s1 = 1;
    float f1 = 1.0f;
    double d1 = 1.0, d15 = 1.5;
    CK(CFEqual(mk(kCFNumberIntType, &i1), mk(kCFNumberLongLongType, &l1)));
    CK(CFEqual(mk(kCFNumberShortType, &s1), mk(kCFNumberLongLongType, &l1)));
    CK(CFEqual(mk(kCFNumberIntType, &i1), mk(kCFNumberFloatType, &f1)));
    CK(CFEqual(mk(kCFNumberDoubleType, &d1), mk(kCFNumberIntType, &i1)));
    CK(!CFEqual(mk(kCFNumberDoubleType, &d15), mk(kCFNumberIntType, &i1)));
    int m1 = -1;
    CK(!CFEqual(mk(kCFNumberIntType, &m1), mk(kCFNumberIntType, &i1)));
    long long big = 0x7fffffffffffffffLL, big2 = 0x7ffffffffffffffeLL;
    CK(CFEqual(mk(kCFNumberLongLongType, &big), mk(kCFNumberLongLongType, &big)));
    CK(!CFEqual(mk(kCFNumberLongLongType, &big), mk(kCFNumberLongLongType, &big2)));
    double nan = __builtin_nan("");
    CK(CFEqual(mk(kCFNumberDoubleType, &nan), mk(kCFNumberDoubleType, &nan)));
    CK(CFEqual(kCFNumberNaN, kCFNumberNaN));
    CK(CFEqual(kCFNumberPositiveInfinity, kCFNumberPositiveInfinity));
    CK(!CFEqual(kCFNumberPositiveInfinity, kCFNumberNegativeInfinity));
    return finish();
}
