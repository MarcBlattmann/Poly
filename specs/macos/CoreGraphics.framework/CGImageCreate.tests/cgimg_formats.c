/* Images of other byte orders and alpha forms drawn into an sRGB bitmap:
 * the pixel [R=0x40 G=0x80 B=0xc0 A=0xff] stored as RGBA, BGRA, ARGB,
 * ABGR, RGBX, and gray; and a 50% alpha pixel, premultiplied and not. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static void draw(const char *label, const unsigned char px[4], CGBitmapInfo info) {
    unsigned char data[4];
    memcpy(data, px, 4);
    CGImageRef img = px_image(data, 1, 1, 4, info, false);
    if (!img) {
        value_str(label, "NULL");
        return;
    }
    pxbuf b = px_rgba(1, 1);
    CGContextDrawImage(b.ctx, CGRectMake(0, 0, 1, 1), img);
    px_value(label, &b, 0, 0);
}

int test_main(void) {
    const unsigned char rgba[4] = {0x40, 0x80, 0xc0, 0xff};
    const unsigned char bgra[4] = {0xc0, 0x80, 0x40, 0xff};
    const unsigned char argb[4] = {0xff, 0x40, 0x80, 0xc0};
    const unsigned char abgr[4] = {0xff, 0xc0, 0x80, 0x40};
    draw("RGBA bytes, premultiplied last", rgba, (CGBitmapInfo)kCGImageAlphaPremultipliedLast);
    draw("BGRA bytes: premultiplied first + 32 little", bgra, (CGBitmapInfo)(kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little));
    draw("ARGB bytes: premultiplied first (default big order)", argb, (CGBitmapInfo)kCGImageAlphaPremultipliedFirst);
    draw("ABGR bytes: premultiplied last + 32 little", abgr, (CGBitmapInfo)(kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Little));
    draw("RGBX bytes, alpha skipped last", rgba, (CGBitmapInfo)kCGImageAlphaNoneSkipLast);
    const unsigned char half_pre[4] = {0x20, 0x40, 0x60, 0x80};
    draw("premultiplied RGBA with alpha 0x80 (stored 20 40 60)", half_pre, (CGBitmapInfo)kCGImageAlphaPremultipliedLast);
    const unsigned char half_non[4] = {0x40, 0x80, 0xc0, 0x80};
    draw("non-premultiplied RGBA with alpha 0x80 (stored 40 80 c0)", half_non, (CGBitmapInfo)kCGImageAlphaLast);
    const unsigned char over[4] = {0xff, 0xff, 0xff, 0x40};
    draw("premultiplied RGBA with a colour above its alpha (ff ff ff, alpha 40)", over, (CGBitmapInfo)kCGImageAlphaPremultipliedLast);
    const unsigned char clear[4] = {0x12, 0x34, 0x56, 0x00};
    draw("non-premultiplied, alpha 0: nothing", clear, (CGBitmapInfo)kCGImageAlphaLast);
    return finish();
}
