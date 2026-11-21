/* poly-mac: appkit */
/* A new window's state: not visible, not key, not main, not miniaturized, title '', alpha 1 (a test
 * helper sets it to 0, so the helper window is checked after resetting), opaque, buffered
 * backing (2), level 0 (normal), minimum size (0, 32) for a titled window, a huge maximum size,
 * a content view that is a plain NSView and the first responder is the window itself, no
 * delegate, no toolbar, a window number above 0, and it is listed by the application. */
#include "poly_mak.h"

int test_main(void) {
    NSApplication *a = ak_init();
    NSWindow *w = [[NSWindow alloc] initWithContentRect:NSMakeRect(0, 0, 200, 100) styleMask:NSWindowStyleMaskTitled backing:NSBackingStoreBuffered defer:NO];
    w.releasedWhenClosed = NO;
    CK(!w.isVisible && !w.isKeyWindow && !w.isMainWindow && !w.isMiniaturized);
    CK(w.title != nil && w.title.length == 0);
    CKF(w.alphaValue, 1);
    CK(w.isOpaque);
    CK(w.backingType == NSBackingStoreBuffered && NSBackingStoreBuffered == 2);
    CK(w.level == NSNormalWindowLevel && NSNormalWindowLevel == 0);
    CKF(w.minSize.width, 0);
    CKF(w.minSize.height, 32);
    CK(w.maxSize.width > 1e6 && w.maxSize.height > 1e6);
    CK(w.contentView != nil && [w.contentView class] == [NSView class]);
    CK(w.firstResponder == w);
    CK(w.delegate == nil && w.toolbar == nil);
    CK(w.windowNumber > 0);
    CK([a.windows containsObject:w]);
    CK(w.canBecomeKeyWindow && !w.canBecomeMainWindow);
    CK(w.contentView.window == w);
    return finish();
}
