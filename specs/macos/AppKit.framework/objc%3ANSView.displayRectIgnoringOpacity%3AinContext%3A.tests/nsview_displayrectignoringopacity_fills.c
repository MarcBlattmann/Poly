/* poly-mac: appkit */
/* Drawing into an sRGB bitmap of the program's own (NOT SYSTEM-DEPENDENT: the bytes are exact): a view of 10 x 8
 * points draws a red (1,0,0,1) fill of (1,1,3,2), a blue (0,0,1,0.5) fill of (6,1,3,2) with source-over and a green fill
 * of (1.5,5.5,3,2) (fractional edges). With scale 1 and the bitmap premultiplied RGBA, row 0 on top: pixels are
 * 0xRRGGBBAA; untouched pixels are 0x00000000; the red fill covers columns 1..3 of rows 5 and 6 (the view is not
 * flipped: y 1..3 is the bottom: 8 - 3 = 5) as 0xff0000ff; the blue one columns 6..8 of rows 5 and 6 as the premultiplied
 * 0x00008080 (alpha 0x80 = 128 of 255 for 0.5, blue scaled the same); the green fill at the fractional edges gets partial
 * coverage: row 0 (the top of the view) and row 2 have half the vertical coverage, column 1 and column 4 half the
 * horizontal: 0x003f003f at the four corners, 0x007f007f on the edges, 0x00ff00ff inside (only row 1 columns 2..3). */
#include "poly_mak.h"

@interface PV : NSView
@end
@implementation PV
- (void)drawRect:(NSRect)r {
    (void)r;
    [[NSColor colorWithSRGBRed:1 green:0 blue:0 alpha:1] setFill];
    NSRectFill(NSMakeRect(1, 1, 3, 2));
    [[NSColor colorWithSRGBRed:0 green:0 blue:1 alpha:0.5] setFill];
    NSRectFillUsingOperation(NSMakeRect(6, 1, 3, 2), NSCompositingOperationSourceOver);
    [[NSColor colorWithSRGBRed:0 green:1 blue:0 alpha:1] setFill];
    NSRectFill(NSMakeRect(1.5, 5.5, 3, 2));
}
@end

int test_main(void) {
    PV *v = [[PV alloc] initWithFrame:NSMakeRect(0, 0, 10, 8)];
    NSBitmapImageRep *r = ak_render(v, ak_light(), 1);
    CK(r.pixelsWide == 10 && r.pixelsHigh == 8 && r.bitsPerPixel == 32 && r.samplesPerPixel == 4 && r.hasAlpha);
    CKV(ak_px(r, 0, 0) == 0x00000000, ak_px(r, 0, 0));
    CKV(ak_px(r, 1, 5) == 0xff0000ff && ak_px(r, 3, 6) == 0xff0000ff, ak_px(r, 1, 5));
    CK(ak_px(r, 0, 5) == 0 && ak_px(r, 4, 5) == 0 && ak_px(r, 1, 4) == 0 && ak_px(r, 1, 7) == 0);
    CKV(ak_px(r, 6, 5) == 0x00008080 && ak_px(r, 8, 6) == 0x00008080, ak_px(r, 6, 5));
    CK(ak_px(r, 5, 5) == 0 && ak_px(r, 9, 5) == 0);
    CKV(ak_px(r, 1, 0) == 0x003f003f && ak_px(r, 4, 0) == 0x003f003f, ak_px(r, 1, 0));
    CKV(ak_px(r, 2, 0) == 0x007f007f && ak_px(r, 1, 1) == 0x007f007f, ak_px(r, 2, 0));
    CKV(ak_px(r, 2, 1) == 0x00ff00ff && ak_px(r, 3, 1) == 0x00ff00ff, ak_px(r, 2, 1));
    CKV(ak_px(r, 1, 2) == 0x003f003f && ak_px(r, 2, 2) == 0x007f007f, ak_px(r, 2, 2));
    ak_dump("rows", r, 0, 0, 10, 8);
    value("hash", (long)(ak_hash(r) & 0x7fffffff));
    return finish();
}
