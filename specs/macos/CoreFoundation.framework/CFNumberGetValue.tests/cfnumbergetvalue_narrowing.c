/* poly-mac: corefoundation */
/* Reading into a narrower integer type wraps modulo its size and still says
 * true; reading a fraction into an integer truncates toward zero and says
 * false; a double out of the integer's range says true and NaN says false,
 * the value stored being the processor's own float-to-integer result (not
 * checked: arm64 saturates, x86_64 gives the smallest integer). */
#include "poly_mcf.h"

static CFNumberRef I(int v) { return CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &v); }
static CFNumberRef D(double v) { return CFNumberCreate(kCFAllocatorDefault, kCFNumberDoubleType, &v); }

int test_main(void) {
    unsigned char uc = 0;
    signed char sc = 0;
    short sh = 0;
    int i = 0;
    long long ll = 0;
    CK(CFNumberGetValue(I(300), kCFNumberCharType, &uc) && uc == 44);
    CK(CFNumberGetValue(I(-129), kCFNumberSInt8Type, &sc) && sc == 127);
    CK(CFNumberGetValue(I(127), kCFNumberSInt8Type, &sc) && sc == 127);
    CK(CFNumberGetValue(I(70000), kCFNumberShortType, &sh) && sh == 4464);
    CK(!CFNumberGetValue(D(2.75), kCFNumberIntType, &i) && i == 2);
    CK(!CFNumberGetValue(D(-2.75), kCFNumberIntType, &i) && i == -2);
    CK(CFNumberGetValue(D(1e30), kCFNumberIntType, &i));
    CK(CFNumberGetValue(D(1e30), kCFNumberLongLongType, &ll));
    CK(!CFNumberGetValue(kCFNumberNaN, kCFNumberIntType, &i));
    float f = 0;
    CK(!CFNumberGetValue(D(0.1), kCFNumberFloatType, &f));
    CK(CFNumberGetValue(D(0.5), kCFNumberFloatType, &f) && f == 0.5f);
    return finish();
}
