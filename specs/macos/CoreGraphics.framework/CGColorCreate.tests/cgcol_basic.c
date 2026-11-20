/* CGColorCreate and the generic constructors: components, alpha, colour
 * space and the constant colours. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static void comps(const char *name, CGColorRef c) {
    char l[100];
    size_t k = 0;
    for (const char *p = name; *p && k < 70; p++) l[k++] = *p;
    l[k++] = ' ';
    l[k + 1] = 0;
    size_t n = CGColorGetNumberOfComponents(c);
    const CGFloat *v = CGColorGetComponents(c);
    for (size_t i = 0; i < n; i++) {
        l[k] = (char)('0' + i);
        value_dbl(l, v[i]);
    }
}

int test_main(void) {
    CGColorRef g = CGColorCreateGenericRGB(0.1, 0.5, 0.9, 0.7);
    check("a generic RGB colour", g != NULL, 0);
    check("4 components (RGB + alpha)", CGColorGetNumberOfComponents(g) == 4, (long long)CGColorGetNumberOfComponents(g));
    comps("generic RGB (0.1 0.5 0.9 0.7)", g);
    value_dbl("alpha", CGColorGetAlpha(g));
    CGColorRef gg = CGColorCreateGenericGray(0.3, 0.6);
    check("generic gray has 2 components", CGColorGetNumberOfComponents(gg) == 2, 0);
    comps("generic gray (0.3, 0.6)", gg);
    CGColorSpaceRef srgb = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGFloat v[4] = {0.2, 0.4, 0.6, 0.8};
    CGColorRef s = CGColorCreate(srgb, v);
    check("the colour space is the one given", CGColorGetColorSpace(s) == srgb, 0);
    comps("sRGB (0.2 0.4 0.6 0.8)", s);
    check("equal colours compare equal", CGColorEqualToColor(s, CGColorCreate(srgb, v)), 0);
    CGColorRef black = CGColorGetConstantColor(kCGColorBlack);
    CGColorRef white = CGColorGetConstantColor(kCGColorWhite);
    CGColorRef clear = CGColorGetConstantColor(kCGColorClear);
    comps("kCGColorBlack", black);
    comps("kCGColorWhite", white);
    comps("kCGColorClear", clear);
    CGFloat w[4] = {2, -1, 0.5, 1};
    comps("a colour made from out-of-range components (2 -1 0.5 1) keeps them", CGColorCreate(srgb, w));
    return finish();
}
