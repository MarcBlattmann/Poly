/* poly-mac: appkit */
/* cacheDisplayInRect:toBitmapImageRep: draws the view (through drawRect: and its subviews) into the bitmap at once:
 * the bitmap's row 0 is the top of the view; untouched pixels stay 0x00000000; a red sRGB fill of (1,1,3,2)
 * in the unflipped 10 x 8 point view comes out OPAQUE (alpha 0xff) at columns 2..7 of rows 10..13 on a display of scale 2
 * (rows 5 and 6 and columns 1..3 at scale 1) but the red, green and blue are CONVERTED into the display's colour
 * profile, so (255,0,0) in sRGB is not 0xff0000 here: SYSTEM-DEPENDENT (this Mac: 0xea3323), and a sRGB 0.2/0.4/0.6
 * fill is 0x406595 (printed). The fills' pixels are all equal inside a fill. The same view drawn under the dark
 * appearance gives the same bytes (the colours are fixed). */
#include "poly_mak.h"

@interface PV : NSView
@end
@implementation PV
- (void)drawRect:(NSRect)r {
    (void)r;
    [[NSColor colorWithSRGBRed:1 green:0 blue:0 alpha:1] setFill];
    NSRectFill(NSMakeRect(1, 1, 3, 2));
    [[NSColor colorWithSRGBRed:0.2 green:0.4 blue:0.6 alpha:1] setFill];
    NSRectFill(NSMakeRect(5, 1, 3, 2));
}
@end

int test_main(void) {
    PV *v = [[PV alloc] initWithFrame:NSMakeRect(0, 0, 10, 8)];
    NSWindow *w = ak_window(NSWindowStyleMaskBorderless, NSMakeRect(0, 0, 10, 8));
    w.contentView = v;
    w.appearance = ak_light();
    int s = (int)w.backingScaleFactor;
    NSBitmapImageRep *r = [v bitmapImageRepForCachingDisplayInRect:v.bounds];
    [v cacheDisplayInRect:v.bounds toBitmapImageRep:r];
    int rx = 1 * s, ry = (8 - 3) * s, bx = 5 * s;
    CK((ak_px(r, rx, ry) & 255) == 255 && (ak_px(r, rx + 3 * s - 1, ry + 2 * s - 1) & 255) == 255);
    CK(ak_px(r, 0, 0) == 0 && ak_px(r, rx - 1, ry) == 0 && ak_px(r, rx, ry - 1) == 0);
    CK(ak_px(r, rx, ry) == ak_px(r, rx + 1, ry + 1));
    CK((ak_px(r, bx, ry) & 255) == 255 && ak_px(r, bx, ry) != ak_px(r, rx, ry));
    value("sys_red_pixel", ak_px(r, rx, ry));
    value("sys_blue_pixel", ak_px(r, bx, ry));
    value("sys_scale", s);
    NSBitmapImageRep *d = [v bitmapImageRepForCachingDisplayInRect:v.bounds];
    w.appearance = ak_dark();
    [v cacheDisplayInRect:v.bounds toBitmapImageRep:d];
    CK(ak_hash(d) == ak_hash(r));
    [w close];
    return finish();
}
