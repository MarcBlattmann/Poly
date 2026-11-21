/* poly-mac: appkit */
/* The instance method agrees with the class method for the window's mask; for a window after
 * setFrame:display: the content view's frame is the content rectangle of the new frame
 * (400 x 300 frame gives a 400 x 268 content view); a frame smaller than the decoration gives a
 * content rectangle of height 0 or less (a 10 high frame has a content height of 0 or less). */
#include "poly_mak.h"

int test_main(void) {
    NSWindow *w = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 100, 100));
    CK(NSEqualRects([w contentRectForFrameRect:NSMakeRect(5, 6, 400, 300)], NSMakeRect(5, 6, 400, 268)));
    CK(NSEqualRects([w contentRectForFrameRect:NSMakeRect(5, 6, 400, 300)], [NSWindow contentRectForFrameRect:NSMakeRect(5, 6, 400, 300) styleMask:NSWindowStyleMaskTitled]));
    [w setFrame:NSMakeRect(-30000, -30000, 400, 300) display:NO];
    CK(NSEqualRects(w.contentView.frame, NSMakeRect(0, 0, 400, 268)));
    NSRect tiny = [NSWindow contentRectForFrameRect:NSMakeRect(0, 0, 50, 10) styleMask:NSWindowStyleMaskTitled];
    CKF(tiny.size.width, 50);
    CK(tiny.size.height <= 0);
    return finish();
}
