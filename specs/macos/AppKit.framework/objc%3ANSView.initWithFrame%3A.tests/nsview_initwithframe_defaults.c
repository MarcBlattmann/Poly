/* poly-mac: appkit */
/* A view made with initWithFrame: keeps the frame exactly (also fractional and negative origins);
 * its bounds start at (0,0) with the frame's size; defaults: not flipped, not hidden, alpha 1, no
 * autoresizing (mask 0), translatesAutoresizingMaskIntoConstraints YES, not needing display, no
 * layer (wantsLayer NO), not opaque, does not accept first responder, tag -1, no superview, no
 * window, no subviews, layerContentsRedrawPolicy NSViewLayerContentsRedrawOnSetNeedsDisplay (1),
 * clipsToBounds NO, wantsUpdateLayer YES for a plain NSView. */
#include "poly_mak.h"

int test_main(void) {
    NSView *v = [[NSView alloc] initWithFrame:NSMakeRect(10, 20, 100, 50)];
    CK(NSEqualRects(v.frame, NSMakeRect(10, 20, 100, 50)));
    CK(NSEqualRects(v.bounds, NSMakeRect(0, 0, 100, 50)));
    CK(!v.isFlipped && !v.isHidden);
    CKF(v.alphaValue, 1);
    CK(v.autoresizingMask == NSViewNotSizable && NSViewNotSizable == 0);
    CK(v.translatesAutoresizingMaskIntoConstraints);
    CK(!v.needsDisplay && !v.wantsLayer && v.layer == nil);
    CK(!v.isOpaque && !v.acceptsFirstResponder);
    CKV(v.tag == -1, v.tag);
    CK(v.superview == nil && v.window == nil && v.subviews.count == 0);
    CK(v.layerContentsRedrawPolicy == NSViewLayerContentsRedrawOnSetNeedsDisplay);
    CK(!v.clipsToBounds && v.wantsUpdateLayer);
    NSView *f = [[NSView alloc] initWithFrame:NSMakeRect(-3.5, 0.25, 7.75, 1.5)];
    CK(NSEqualRects(f.frame, NSMakeRect(-3.5, 0.25, 7.75, 1.5)));
    CK(NSEqualRects(f.bounds, NSMakeRect(0, 0, 7.75, 1.5)));
    return finish();
}
