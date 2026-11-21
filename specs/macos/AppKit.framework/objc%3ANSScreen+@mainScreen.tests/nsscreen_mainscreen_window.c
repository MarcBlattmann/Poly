/* poly-mac: appkit */
/* How windows relate to the main screen: a window's screen is the main screen (also before it is shown);
 * a window's backingScaleFactor is the screen's; setFrameOrigin: to (0,0) puts the frame there and a frame wider than the
 * screen is kept (setFrame:display: does not clip); constrainFrameRect:toScreen: moves a titled window that lies far below the
 * screen back up (its y grows so the title bar can be reached; x unchanged), and leaves a borderless window's
 * frame alone (-5000 stays). */
#include "poly_mak.h"

int test_main(void) {
    NSWindow *w = ak_window(NSWindowStyleMaskTitled, NSMakeRect(100, 100, 200, 100));
    NSScreen *s = NSScreen.mainScreen;
    CK(w.screen == s);
    CKF(w.backingScaleFactor, s.backingScaleFactor);
    [w setFrameOrigin:NSMakePoint(0, 0)];
    CK(w.frame.origin.x == 0 && w.frame.origin.y == 0);
    [w setFrame:NSMakeRect(0, 0, s.frame.size.width + 500, 100) display:NO];
    CKF(w.frame.size.width, s.frame.size.width + 500);
    NSRect far = NSMakeRect(100, -5000, 200, 132);
    NSRect fixed = [w constrainFrameRect:far toScreen:s];
    CKF(fixed.origin.x, 100);
    CK(fixed.origin.y > far.origin.y);
    value_f("sys_constrained_y_delta_sign", fixed.origin.y > far.origin.y ? 1 : 0);
    NSWindow *b = ak_window(NSWindowStyleMaskBorderless, NSMakeRect(0, 0, 50, 50));
    NSRect same = [b constrainFrameRect:NSMakeRect(100, -5000, 50, 50) toScreen:s];
    value_f("borderless constrained y", same.origin.y);
    return finish();
}
