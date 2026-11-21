/* poly-mac: appkit */
/* releasedWhenClosed defaults to YES for a window made in code; the test sets it to NO to keep using the window;
 * a closed window can be ordered front again and is then visible, and its content view and title are
 * unchanged. isReleasedWhenClosed reads back what was set. */
#include "poly_mak.h"

int test_main(void) {
    ak_init();
    NSWindow *w = [[NSWindow alloc] initWithContentRect:NSMakeRect(0, 0, 100, 100) styleMask:NSWindowStyleMaskTitled backing:NSBackingStoreBuffered defer:NO];
    w.alphaValue = 0;
    w.ignoresMouseEvents = YES;
    value("isReleasedWhenClosed (default)", w.isReleasedWhenClosed);
    w.releasedWhenClosed = NO;
    CK(!w.isReleasedWhenClosed);
    NSView *cv = w.contentView;
    w.title = @"again";
    [w orderFront:nil];
    ak_spin(0.05);
    [w close];
    CK(!w.isVisible);
    [w orderFront:nil];
    ak_spin(0.05);
    CK(w.isVisible);
    CK(w.contentView == cv && [w.title isEqualToString:@"again"]);
    [w close];
    return finish();
}
