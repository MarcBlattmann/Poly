/* poly-mac: appkit */
/* Window coordinates (the view argument nil) always start at the bottom left of the window's content area,
 * whatever the flippedness of the views: a plain subview at (10,10) of size 50 x 20 in a window of content
 * height 100 converts its own (0,0) to window (10,10); a flipped one of the same frame converts its (0,0)
 * to window (10,30), because its (0,0) is the top left corner; its (0,20) (the bottom left in flipped
 * coordinates) is at window y 10; converting window points back with convertPoint:fromView:nil reverses
 * this; a point is converted by the same rules in a subview of a flipped superview. */
#include "poly_mak.h"

@interface FV : NSView
@end
@implementation FV
- (BOOL)isFlipped { return YES; }
@end

int test_main(void) {
    NSWindow *w = ak_window(NSWindowStyleMaskBorderless, NSMakeRect(0, 0, 200, 100));
    NSView *p = [[NSView alloc] initWithFrame:NSMakeRect(10, 10, 50, 20)];
    FV *f = [[FV alloc] initWithFrame:NSMakeRect(10, 10, 50, 20)];
    [w.contentView addSubview:p];
    [w.contentView addSubview:f];
    NSPoint a = [p convertPoint:NSMakePoint(0, 0) toView:nil];
    CKF(a.x, 10);
    CKF(a.y, 10);
    NSPoint b = [f convertPoint:NSMakePoint(0, 0) toView:nil];
    CKF(b.x, 10);
    CKF(b.y, 30);
    NSPoint c = [f convertPoint:NSMakePoint(0, 20) toView:nil];
    CKF(c.y, 10);
    NSPoint d = [f convertPoint:NSMakePoint(10, 30) fromView:nil];
    CKF(d.x, 0);
    CKF(d.y, 0);
    NSPoint e = [p convertPoint:NSMakePoint(10, 10) fromView:nil];
    CKF(e.x, 0);
    CKF(e.y, 0);
    CK(NSEqualRects([f convertRect:f.bounds toView:nil], NSMakeRect(10, 10, 50, 20)));
    return finish();
}
