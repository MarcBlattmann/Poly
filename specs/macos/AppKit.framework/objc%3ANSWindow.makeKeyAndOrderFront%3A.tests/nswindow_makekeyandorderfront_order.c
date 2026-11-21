/* poly-mac: appkit */
/* The ordering calls of a window and visibility: orderFront:, orderBack:, orderFrontRegardless and
 * makeKeyAndOrderFront: each make a window visible, ordering one window front does not change
 * another's visibility, orderOut: hides only the window it is sent to, orderWindow:relativeTo: with
 * NSWindowAbove/NSWindowBelow and a window number of 0 orders front/back and shows the window, and
 * ordering a window that is already visible keeps it visible. NSWindowAbove is 1, NSWindowBelow -1, NSWindowOut 0. */
#include "poly_mak.h"

int test_main(void) {
    NSWindow *w1 = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 100, 100));
    NSWindow *w2 = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 100, 100));
    NSWindow *w3 = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 100, 100));
    CK(NSWindowAbove == 1 && NSWindowBelow == -1 && NSWindowOut == 0);
    [w1 orderFront:nil];
    CK(w1.isVisible && !w2.isVisible && !w3.isVisible);
    [w2 orderBack:nil];
    CK(w1.isVisible && w2.isVisible && !w3.isVisible);
    [w3 orderFrontRegardless];
    CK(w3.isVisible);
    [w1 orderFront:nil];
    CK(w1.isVisible && w2.isVisible && w3.isVisible);
    [w2 orderOut:nil];
    CK(w1.isVisible && !w2.isVisible && w3.isVisible);
    [w2 orderWindow:NSWindowAbove relativeTo:0];
    CK(w2.isVisible);
    [w1 orderOut:nil];
    [w1 orderWindow:NSWindowBelow relativeTo:w3.windowNumber];
    ak_spin(0.1);
    CK(w1.isVisible);
    [w1 orderWindow:NSWindowOut relativeTo:0];
    CK(!w1.isVisible);
    [w1 close]; [w2 close]; [w3 close];
    return finish();
}
