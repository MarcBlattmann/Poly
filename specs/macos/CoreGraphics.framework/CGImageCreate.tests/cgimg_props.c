/* CGImageCreate: the image keeps the size, depth, row bytes, bitmap info
 * and colour space it was made with; bad arguments give NULL. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static unsigned char pix[4 * 3 * 4];

int test_main(void) {
    for (int i = 0; i < 48; i++) pix[i] = (unsigned char)(i * 5 + 1);
    CGImageRef img = px_image(pix, 4, 3, 16, (CGBitmapInfo)kCGImageAlphaPremultipliedLast, false);
    check("an image is made", img != NULL, 0);
    check("width 4", CGImageGetWidth(img) == 4, 0);
    check("height 3", CGImageGetHeight(img) == 3, 0);
    check("8 bits per component", CGImageGetBitsPerComponent(img) == 8, 0);
    check("32 bits per pixel", CGImageGetBitsPerPixel(img) == 32, 0);
    check("16 bytes per row", CGImageGetBytesPerRow(img) == 16, 0);
    check("bitmap info is what was given", CGImageGetBitmapInfo(img) == (CGBitmapInfo)kCGImageAlphaPremultipliedLast, (long long)CGImageGetBitmapInfo(img));
    check("alpha info", CGImageGetAlphaInfo(img) == kCGImageAlphaPremultipliedLast, 0);
    check("not a mask", !CGImageIsMask(img), 0);
    check("no decode array", CGImageGetDecode(img) == NULL, 0);
    check("should interpolate is as given (false)", !CGImageGetShouldInterpolate(img), 0);
    value("rendering intent", CGImageGetRenderingIntent(img));
    check("the colour space is sRGB's (same object)", CGImageGetColorSpace(img) == CGColorSpaceCreateWithName(kCGColorSpaceSRGB), 0);
    CFDataRef d = CGDataProviderCopyData(CGImageGetDataProvider(img));
    check("the data provider gives the 48 bytes back", d && CFDataGetLength(d) == 48 && CFDataGetBytePtr(d)[7] == pix[7], 0);
    if (d) CFRelease(d);
    check("an image with interpolate = true reports it", CGImageGetShouldInterpolate(px_image(pix, 4, 3, 16, (CGBitmapInfo)kCGImageAlphaPremultipliedLast, true)), 0);

    CGDataProviderRef prov = CGDataProviderCreateWithData(NULL, pix, sizeof pix, NULL);
    CGColorSpaceRef cs = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    check("width 0: NULL", CGImageCreate(0, 3, 8, 32, 16, cs, kCGImageAlphaPremultipliedLast, prov, NULL, false, kCGRenderingIntentDefault) == NULL, 0);
    check("bytes per row too small: NULL", CGImageCreate(4, 3, 8, 32, 8, cs, kCGImageAlphaPremultipliedLast, prov, NULL, false, kCGRenderingIntentDefault) == NULL, 0);
    check("bits per pixel not a multiple of bits per component: NULL", CGImageCreate(4, 3, 8, 30, 16, cs, kCGImageAlphaPremultipliedLast, prov, NULL, false, kCGRenderingIntentDefault) == NULL, 0);
    check("a NULL provider: NULL", CGImageCreate(4, 3, 8, 32, 16, cs, kCGImageAlphaPremultipliedLast, NULL, NULL, false, kCGRenderingIntentDefault) == NULL, 0);
    check("a NULL colour space (not a mask): NULL", CGImageCreate(4, 3, 8, 32, 16, NULL, kCGImageAlphaPremultipliedLast, prov, NULL, false, kCGRenderingIntentDefault) == NULL, 0);
    return finish();
}
