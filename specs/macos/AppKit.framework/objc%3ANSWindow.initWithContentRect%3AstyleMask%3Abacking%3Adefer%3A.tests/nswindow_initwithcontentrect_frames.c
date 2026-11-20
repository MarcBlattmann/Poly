/* poly-mac: appkit */
/* The frame of a new window from a 300 x 200 content rectangle at (100,200): a borderless window's
 * frame is the content rectangle itself; a titled window's frame is 32 higher (the title bar of this
 * macOS, 232) with the same width and origin; with the full-size content view flag the frame is
 * the content rectangle (200) while the layout rectangle is 168 high; a window with only the
 * resizable flag has no title bar (200); the content view fills the content rectangle at (0,0);
 * the style mask reads back as given. */
#include "poly_mak.h"

static NSWindow *mk(NSWindowStyleMask m) { return ak_window(m, NSMakeRect(100, 200, 300, 200)); }

int test_main(void) {
    NSWindow *b = mk(NSWindowStyleMaskBorderless);
    CK(NSEqualRects(b.frame, NSMakeRect(100, 200, 300, 200)));
    CK(NSEqualRects(b.contentView.frame, NSMakeRect(0, 0, 300, 200)));
    NSWindow *t = mk(NSWindowStyleMaskTitled);
    CK(NSEqualRects(t.frame, NSMakeRect(100, 200, 300, 232)));
    CK(NSEqualRects(t.contentView.frame, NSMakeRect(0, 0, 300, 200)));
    CK(t.styleMask == NSWindowStyleMaskTitled);
    NSWindow *all = mk(NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable);
    CK(NSEqualRects(all.frame, NSMakeRect(100, 200, 300, 232)));
    CK(all.styleMask == (NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable));
    NSWindow *f = mk(NSWindowStyleMaskTitled | NSWindowStyleMaskFullSizeContentView);
    CK(NSEqualRects(f.frame, NSMakeRect(100, 200, 300, 200)));
    CK(NSEqualRects(f.contentLayoutRect, NSMakeRect(0, 0, 300, 168)));
    NSWindow *r = mk(NSWindowStyleMaskResizable);
    CK(NSEqualRects(r.frame, NSMakeRect(100, 200, 300, 200)));
    return finish();
}
