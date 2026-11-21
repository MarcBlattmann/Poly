/* poly-mac: appkit */
/* A window's `screen` is the screen it is mostly on: it is not nil even before the window is ordered
 * onto the screen, and the same after ordering front; the screen's frame has a positive size and
 * its visibleFrame is inside the frame; the window's colorSpace is not nil (the screen's). */
#include "poly_mak.h"

int test_main(void) {
    NSWindow *w = ak_window(NSWindowStyleMaskTitled, NSMakeRect(100, 100, 100, 100));
    CK(w.screen != nil);
    [w orderFront:nil];
    ak_spin(0.1);
    CK(w.screen != nil);
    CK(w.screen.frame.size.width > 0 && w.screen.frame.size.height > 0);
    NSRect f = w.screen.frame, v = w.screen.visibleFrame;
    CK(NSContainsRect(f, v));
    CK(NSScreen.screens.count >= 1);
    CK(NSScreen.screens[0] == NSScreen.mainScreen || NSScreen.mainScreen != nil);
    CK(w.colorSpace != nil);
    [w close];
    return finish();
}
