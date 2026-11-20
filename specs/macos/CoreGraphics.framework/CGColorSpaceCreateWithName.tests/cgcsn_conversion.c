/* The same fill (one colour given in sRGB, set by a CGColor made in each
 * named space) drawn into an sRGB bitmap: the stored bytes show the
 * conversion between spaces. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static void one(const char *label, CFStringRef space, double r, double g, double bl) {
    CGColorSpaceRef cs = CGColorSpaceCreateWithName(space);
    CGFloat comp[4] = {(CGFloat)r, (CGFloat)g, (CGFloat)bl, 1};
    CGColorRef c = CGColorCreate(cs, comp);
    pxbuf b = px_rgba(1, 1);
    CGContextSetFillColorWithColor(b.ctx, c);
    CGContextFillRect(b.ctx, CGRectMake(0, 0, 1, 1));
    px_value(label, &b, 0, 0);
    CGColorRelease(c);
}

int test_main(void) {
    one("sRGB (0.2, 0.4, 0.6) into sRGB", kCGColorSpaceSRGB, 0.2, 0.4, 0.6);
    one("DisplayP3 (0.2, 0.4, 0.6) into sRGB", kCGColorSpaceDisplayP3, 0.2, 0.4, 0.6);
    one("DisplayP3 (1, 0, 0) into sRGB (out of gamut: clamped)", kCGColorSpaceDisplayP3, 1, 0, 0);
    one("GenericRGB (0.2, 0.4, 0.6) into sRGB", kCGColorSpaceGenericRGB, 0.2, 0.4, 0.6);
    one("LinearSRGB (0.2, 0.4, 0.6) into sRGB", kCGColorSpaceLinearSRGB, 0.2, 0.4, 0.6);
    one("ExtendedSRGB (1.2, -0.1, 0.5) into sRGB (clamped)", kCGColorSpaceExtendedSRGB, 1.2, -0.1, 0.5);
    one("sRGB white", kCGColorSpaceSRGB, 1, 1, 1);
    one("DisplayP3 white", kCGColorSpaceDisplayP3, 1, 1, 1);
    return finish();
}
