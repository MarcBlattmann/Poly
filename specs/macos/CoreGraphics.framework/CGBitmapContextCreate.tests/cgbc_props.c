/* CGBitmapContextCreate: the accessors report what was asked, the buffer
 * is used in place (no copy), row bytes may be larger than needed, and bad
 * arguments make NULL. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static unsigned char mem[128 * 128 * 4];

int test_main(void) {
    CGColorSpaceRef rgb = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGContextRef c = CGBitmapContextCreate(mem, 20, 10, 8, 128, rgb, kCGImageAlphaPremultipliedLast);
    check("a context with extra row bytes (128 for 80 needed) is made", c != NULL, 0);
    check("width", CGBitmapContextGetWidth(c) == 20, (long long)CGBitmapContextGetWidth(c));
    check("height", CGBitmapContextGetHeight(c) == 10, (long long)CGBitmapContextGetHeight(c));
    check("bits per component", CGBitmapContextGetBitsPerComponent(c) == 8, 0);
    check("bits per pixel", CGBitmapContextGetBitsPerPixel(c) == 32, (long long)CGBitmapContextGetBitsPerPixel(c));
    check("bytes per row is the one given", CGBitmapContextGetBytesPerRow(c) == 128, (long long)CGBitmapContextGetBytesPerRow(c));
    check("the data pointer is the buffer given", CGBitmapContextGetData(c) == mem, 0);
    check("alpha info", CGBitmapContextGetAlphaInfo(c) == kCGImageAlphaPremultipliedLast, CGBitmapContextGetAlphaInfo(c));
    check("bitmap info is the alpha value", CGBitmapContextGetBitmapInfo(c) == (CGBitmapInfo)kCGImageAlphaPremultipliedLast, (long long)CGBitmapContextGetBitmapInfo(c));
    check("the colour space is the one given", CGBitmapContextGetColorSpace(c) == rgb, 0);
    CGContextSetRGBFillColor(c, 1, 1, 1, 1);
    CGContextFillRect(c, CGRectMake(0, 0, 20, 10));
    int stray = 0;
    for (int row = 0; row < 10; row++)
        for (int i = 80; i < 128; i++) stray += mem[row * 128 + i] != 0;
    check("the padding after each row (bytes 80..127) is not written", stray == 0, stray);
    check("the pixels are in the buffer given", mem[0] == 255 && mem[79] == 255 && mem[128] == 255, mem[0]);

    check("a bytes-per-row smaller than needed gives NULL", CGBitmapContextCreate(mem, 20, 10, 8, 40, rgb, kCGImageAlphaPremultipliedLast) == NULL, 0);
    check("width 0 gives NULL", CGBitmapContextCreate(mem, 0, 10, 8, 80, rgb, kCGImageAlphaPremultipliedLast) == NULL, 0);
    check("height 0 gives NULL", CGBitmapContextCreate(mem, 10, 0, 8, 80, rgb, kCGImageAlphaPremultipliedLast) == NULL, 0);
    check("a NULL colour space without alpha-only gives NULL", CGBitmapContextCreate(mem, 4, 4, 8, 16, NULL, kCGImageAlphaPremultipliedLast) == NULL, 0);
    check("an unsupported alpha with RGB (kCGImageAlphaOnly) gives NULL", CGBitmapContextCreate(mem, 4, 4, 8, 16, rgb, kCGImageAlphaOnly) == NULL, 0);
    check("an unsupported 3 bits per component gives NULL", CGBitmapContextCreate(mem, 4, 4, 3, 16, rgb, kCGImageAlphaPremultipliedLast) == NULL, 0);
    return finish();
}
