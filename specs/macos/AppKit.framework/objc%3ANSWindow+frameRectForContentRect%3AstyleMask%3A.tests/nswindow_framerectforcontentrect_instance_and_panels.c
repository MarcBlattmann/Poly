/* poly-mac: appkit */
/* The instance method gives the same as the class method for the window's own mask (a titled
 * window: 232 for 200); the class method for the panel-only masks differs from a plain window's
 * frame: the utility mask gives 224, the HUD mask 219 (the title bar of those panels is
 * thinner), though an NSWindow (not an NSPanel) with such a mask still gets the 32 point title
 * bar; the full-size mask never changes the width. */
#include "poly_mak.h"

int test_main(void) {
    NSWindow *t = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 300, 200));
    CK(NSEqualRects([t frameRectForContentRect:NSMakeRect(0, 0, 300, 200)], NSMakeRect(0, 0, 300, 232)));
    CK(NSEqualRects([t frameRectForContentRect:NSMakeRect(7, 9, 50, 60)], [NSWindow frameRectForContentRect:NSMakeRect(7, 9, 50, 60) styleMask:NSWindowStyleMaskTitled]));
    NSRect c = NSMakeRect(100, 200, 300, 200);
    CK(NSEqualRects([NSWindow frameRectForContentRect:c styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskUtilityWindow], NSMakeRect(100, 200, 300, 224)));
    CK(NSEqualRects([NSWindow frameRectForContentRect:c styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskHUDWindow], NSMakeRect(100, 200, 300, 219)));
    NSWindow *u = ak_window(NSWindowStyleMaskTitled | NSWindowStyleMaskUtilityWindow, c);
    CKF(u.frame.size.height, 232);
    CKF([NSWindow frameRectForContentRect:c styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskUnifiedTitleAndToolbar].size.height, 232);
    return finish();
}
