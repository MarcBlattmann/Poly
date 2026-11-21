/* poly-mac: appkit */
/* wantsLayer is NO for a new view and it has no layer; setting it to YES creates the backing layer at once
 * (`layer` is not nil, a private subclass of CALayer, and the view is the layer's delegate), the layer's bounds
 * follow the view's size (40 x 30, then 80 x 90 after a resize), canDrawSubviewsIntoLayer is NO and the
 * layer is not opaque; setting wantsLayer back to NO removes the layer (layer is nil); a plain NSView
 * has layerContentsRedrawPolicy OnSetNeedsDisplay (1), a view with its own drawRect: DuringViewResize (2),
 * both before and after wantsLayer; wantsUpdateLayer is YES for a plain NSView and NO for a subclass that
 * overrides drawRect:. */
#include "poly_mak.h"

@interface DV : NSView
@end
@implementation DV
- (void)drawRect:(NSRect)r { (void)r; }
@end

int test_main(void) {
    NSView *v = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 40, 30)];
    CK(!v.wantsLayer && v.layer == nil);
    v.wantsLayer = YES;
    CK(v.wantsLayer && v.layer != nil);
    CK(![v.layer isMemberOfClass:[CALayer class]] && [v.layer isKindOfClass:[CALayer class]]);
    CK((id)v.layer.delegate == (id)v);
    CKF(v.layer.bounds.size.width, 40);
    CKF(v.layer.bounds.size.height, 30);
    CK(!v.canDrawSubviewsIntoLayer && !v.layer.isOpaque);
    v.frame = NSMakeRect(5, 6, 80, 90);
    CKF(v.layer.bounds.size.width, 80);
    CKF(v.layer.bounds.size.height, 90);
    v.wantsLayer = NO;
    CK(!v.wantsLayer && v.layer == nil);
    DV *dv = [[DV alloc] initWithFrame:NSMakeRect(0, 0, 10, 10)];
    CK(v.wantsUpdateLayer && !dv.wantsUpdateLayer);
    CK(v.layerContentsRedrawPolicy == NSViewLayerContentsRedrawOnSetNeedsDisplay);
    CK(dv.layerContentsRedrawPolicy == NSViewLayerContentsRedrawDuringViewResize);
    dv.wantsLayer = YES;
    CK(dv.layerContentsRedrawPolicy == NSViewLayerContentsRedrawDuringViewResize);
    return finish();
}
