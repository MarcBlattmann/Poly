/* poly-mac: appkit */
/* The same drawing in a view whose isFlipped is YES (not SYSTEM-DEPENDENT): the origin is at the top, so y 1..3
 * is rows 1 and 2 (not 5 and 6) and the fractional green fill of (1.5,5.5,3,2) lies in rows 5, 6 and 7 with the same
 * coverage values as before (0x003f003f at the corners of the edge rows, 0x00ff00ff in the middle); the bitmap
 * is the vertical mirror of the unflipped one: pixel (x, y) of the flipped bitmap equals pixel (x, 7 - y) of the unflipped
 * one for every pixel. */
#include "poly_mak.h"

@interface PV : NSView
@property(nonatomic) BOOL flip;
@end
@implementation PV
- (BOOL)isFlipped { return self.flip; }
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
    PV *f = [[PV alloc] initWithFrame:NSMakeRect(0, 0, 10, 8)];
    f.flip = YES;
    NSBitmapImageRep *r = ak_render(f, ak_light(), 1);
    PV *u = [[PV alloc] initWithFrame:NSMakeRect(0, 0, 10, 8)];
    NSBitmapImageRep *q = ak_render(u, ak_light(), 1);
    CKV(ak_px(r, 1, 1) == 0xff0000ff && ak_px(r, 3, 2) == 0xff0000ff, ak_px(r, 1, 1));
    CK(ak_px(r, 1, 0) == 0 && ak_px(r, 1, 3) == 0);
    CKV(ak_px(r, 1, 5) == 0x003f003f && ak_px(r, 2, 6) == 0x00ff00ff && ak_px(r, 4, 7) == 0x003f003f, ak_px(r, 2, 6));
    int mirror = 1;
    for (int y = 0; y < 8; y++)
        for (int x = 0; x < 10; x++) mirror &= ak_px(r, x, y) == ak_px(q, x, 7 - y);
    CK(mirror);
    ak_dump("rows", r, 0, 0, 10, 8);
    value("hash", (long)(ak_hash(r) & 0x7fffffff));
    return finish();
}
