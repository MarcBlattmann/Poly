/* poly-mac: appkit */
/* performAsCurrentDrawingAppearance: runs the block with currentDrawingAppearance set to that appearance and
 * puts the previous one back afterwards (nested calls too: dark inside light inside dark); inside it, a catalog colour
 * resolves for that appearance (labelColor is white in dark, black in light: the red component is above 0.9 and below
 * 0.1); the colour cache does not leak out (after the block the same catalog colour resolves with the earlier
 * appearance); drawing an NSView (displayRectIgnoringOpacity:inContext:) uses the appearance of the view's window
 * for catalog colours. */
#include "poly_mak.h"

static double red(NSColor *c) { return [c colorUsingColorSpace:[NSColorSpace sRGBColorSpace]].redComponent; }

@interface LV : NSView
@end
@implementation LV
- (void)drawRect:(NSRect)r {
    [NSColor.labelColor setFill];
    NSRectFill(r);
}
@end

int test_main(void) {
    __block NSAppearance *inside = nil;
    __block double r1 = -1, r2 = -1, r3 = -1;
    NSAppearance *before = NSAppearance.currentDrawingAppearance;
    [ak_dark() performAsCurrentDrawingAppearance:^{
        inside = NSAppearance.currentDrawingAppearance;
        r1 = red(NSColor.labelColor);
        [ak_light() performAsCurrentDrawingAppearance:^{
            r2 = red(NSColor.labelColor);
        }];
        r3 = red(NSColor.labelColor);
    }];
    CK(inside == ak_dark());
    CK(r1 > 0.9 && r2 < 0.1 && r3 > 0.9);
    CK(NSAppearance.currentDrawingAppearance == before);
    LV *v = [[LV alloc] initWithFrame:NSMakeRect(0, 0, 4, 4)];
    NSBitmapImageRep *d = ak_render(v, ak_dark(), 1);
    LV *u = [[LV alloc] initWithFrame:NSMakeRect(0, 0, 4, 4)];
    NSBitmapImageRep *l = ak_render(u, ak_light(), 1);
    unsigned pd = ak_px(d, 1, 1), pl = ak_px(l, 1, 1);
    CK((pd >> 24) > 200 && (pl >> 24) < 40);
    value("sys_dark_px", pd);
    value("sys_light_px", pl);
    return finish();
}
