/* poly-mac: appkit */
/* A view whose wantsUpdateLayer returns YES is not asked to draw: in a shown window AppKit calls updateLayer
 * instead of drawRect: (once for the first draw, and again on the next turn after needsDisplay is set); a view
 * with only drawRect: gets drawRect:, also when wantsLayer is YES; updateLayer is called with the view's
 * layer in place (`layer` is not nil inside it). */
#include "poly_mak.h"

static NSMutableString *g_log;
static int g_has_layer;
@interface UV : NSView
@end
@implementation UV
- (BOOL)wantsUpdateLayer { return YES; }
- (void)updateLayer {
    [g_log appendString:@"updateLayer;"];
    g_has_layer = self.layer != nil;
    self.layer.backgroundColor = NSColor.redColor.CGColor;
}
- (void)drawRect:(NSRect)r { (void)r; [g_log appendString:@"drawRect;"]; }
@end
@interface DV : NSView
@end
@implementation DV
- (void)drawRect:(NSRect)r { (void)r; [g_log appendString:@"drawRect;"]; }
@end

int test_main(void) {
    g_log = [NSMutableString string];
    UV *u = [[UV alloc] initWithFrame:NSMakeRect(0, 0, 40, 30)];
    NSWindow *w = ak_window(NSWindowStyleMaskBorderless, NSMakeRect(0, 0, 40, 30));
    w.contentView = u;
    [w orderFront:nil];
    ak_spin(0.3);
    CK([g_log rangeOfString:@"updateLayer;"].location != NSNotFound);
    CK([g_log rangeOfString:@"drawRect;"].location == NSNotFound);
    CK(g_has_layer);
    [g_log setString:@""];
    u.needsDisplay = YES;
    ak_spin(0.2);
    CK([g_log isEqualToString:@"updateLayer;"]);
    [w close];
    [g_log setString:@""];
    DV *dd = [[DV alloc] initWithFrame:NSMakeRect(0, 0, 40, 30)];
    dd.wantsLayer = YES;
    NSWindow *w3 = ak_window(NSWindowStyleMaskBorderless, NSMakeRect(0, 0, 40, 30));
    w3.contentView = dd;
    [w3 orderFront:nil];
    ak_spin(0.3);
    CK([g_log rangeOfString:@"drawRect;"].location != NSNotFound);
    [w3 close];
    return finish();
}
