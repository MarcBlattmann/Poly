/* poly-mac: appkit */
/* Inside drawRect: the current graphics context exists and is a window context (not a bitmap):
 * `NSGraphicsContext.currentContext` is not nil, its isFlipped equals the view's isFlipped (NO for a plain
 * view, YES for one that returns YES), isDrawingToScreen is YES, and its CGContext is not NULL;
 * `NSGraphicsContext.currentContext.isDrawingToScreen` is NO while drawing into a bitmap with
 * cacheDisplayInRect; the `NSView.focusView` exists (-[NSView focusView] is deprecated and only the
 * public NSGraphicsContext is used). `needsToDrawRect:` is YES for the dirty rectangle and NO for one
 * outside of it. */
#include "poly_mak.h"

static int g_ctx, g_flip, g_screen, g_cg, g_need_in, g_need_out, g_n;
@interface DV : NSView
@property(nonatomic) BOOL flip;
@end
@implementation DV
- (BOOL)isFlipped { return self.flip; }
- (void)drawRect:(NSRect)r {
    NSGraphicsContext *c = NSGraphicsContext.currentContext;
    g_ctx = c != nil;
    g_flip = c.isFlipped == self.isFlipped;
    g_screen = c.isDrawingToScreen;
    g_cg = c.CGContext != NULL;
    g_need_in = [self needsToDrawRect:NSMakeRect(1, 1, 5, 5)];
    g_need_out = [self needsToDrawRect:NSMakeRect(500, 500, 5, 5)];
    (void)r;
    g_n++;
}
@end

int test_main(void) {
    for (int flip = 0; flip < 2; flip++) {
        g_n = 0;
        DV *d = [[DV alloc] initWithFrame:NSMakeRect(0, 0, 40, 40)];
        d.flip = flip;
        NSWindow *w = ak_window(NSWindowStyleMaskBorderless, NSMakeRect(0, 0, 40, 40));
        w.contentView = d;
        [w orderFront:nil];
        ak_spin(0.3);
        CK(g_n >= 1);
        CK(g_ctx && g_flip && g_cg);
        CK(g_screen);
        CK(g_need_in && !g_need_out);
        [w close];
    }
    return finish();
}
