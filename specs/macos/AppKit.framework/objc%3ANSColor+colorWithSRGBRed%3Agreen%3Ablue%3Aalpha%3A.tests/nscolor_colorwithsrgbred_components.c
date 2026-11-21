/* poly-mac: appkit */
/* +colorWithSRGBRed:green:blue:alpha: keeps its components exactly (1, 0.5, 0.25, 0.75 read back), is in
 * the sRGB colour space ([NSColorSpace sRGBColorSpace], 4 components, type NSColorTypeComponentBased 0) and
 * is not premultiplied; two such colours with equal components are equal (isEqual:), unequal if one component
 * differs; the alpha defaults are exact (colour made with alpha 1 reads 1); getHue:saturation:brightness:alpha:
 * of pure red is 0, 1, 1, 1 (the hue read here is 1.0 or 0.0, printed); components outside 0..1 are
 * kept as given (extended range: -0.5 and 2 read back). */
#include "poly_mak.h"

int test_main(void) {
    NSColor *c = [NSColor colorWithSRGBRed:1 green:0.5 blue:0.25 alpha:0.75];
    CKF(c.redComponent, 1);
    CKF(c.greenComponent, 0.5);
    CKF(c.blueComponent, 0.25);
    CKF(c.alphaComponent, 0.75);
    CK(c.colorSpace == [NSColorSpace sRGBColorSpace] && c.numberOfComponents == 4);
    CK(c.type == NSColorTypeComponentBased && NSColorTypeComponentBased == 0);
    CK([c isEqual:[NSColor colorWithSRGBRed:1 green:0.5 blue:0.25 alpha:0.75]]);
    CK(![c isEqual:[NSColor colorWithSRGBRed:1 green:0.5 blue:0.25 alpha:0.5]]);
    CK(![c isEqual:[NSColor colorWithSRGBRed:0.9 green:0.5 blue:0.25 alpha:0.75]]);
    CKF([NSColor colorWithSRGBRed:0 green:0 blue:0 alpha:1].alphaComponent, 1);
    CGFloat h = -1, s = -1, b = -1, a = -1;
    [[NSColor colorWithSRGBRed:1 green:0 blue:0 alpha:1] getHue:&h saturation:&s brightness:&b alpha:&a];
    CKF(s, 1);
    CKF(b, 1);
    CKF(a, 1);
    CK(h < 0.01 || h > 0.99);
    NSColor *x = [NSColor colorWithSRGBRed:-0.5 green:2 blue:0 alpha:1];
    CKF(x.redComponent, -0.5);
    CKF(x.greenComponent, 2);
    return finish();
}
