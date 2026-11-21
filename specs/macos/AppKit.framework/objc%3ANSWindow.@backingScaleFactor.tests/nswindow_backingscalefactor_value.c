/* poly-mac: appkit */
/* backingScaleFactor of a window on the Mac's built-in or main display is the display's scale: 1.0 or 2.0 depending on
 * the display (this Mac: SYSTEM-DEPENDENT, see the value); it is the same for the content view's
 * convertRectToBacking: (a 10 x 10 rectangle gives 10 x factor); it is the same value as the main
 * screen's, and a window that is not on the screen yet reports the main screen's factor. */
#include "poly_mak.h"

int test_main(void) {
    NSWindow *w = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 100, 100));
    double f = w.backingScaleFactor;
    CK(f == 1.0 || f == 2.0 || f == 3.0);
    value_f("sys_scale", f);
    CKF(w.contentView.window.backingScaleFactor, f);
    NSRect r = [w.contentView convertRectToBacking:NSMakeRect(0, 0, 10, 10)];
    CKF(r.size.width, 10 * f);
    CKF(NSScreen.mainScreen.backingScaleFactor, f);
    return finish();
}
