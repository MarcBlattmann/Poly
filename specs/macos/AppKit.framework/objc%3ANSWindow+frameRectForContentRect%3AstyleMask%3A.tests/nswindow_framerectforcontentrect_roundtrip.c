/* poly-mac: appkit */
/* frameRectForContentRect: and contentRectForFrameRect: are inverse for every mask above: for a
 * grid of content rectangles the frame of the content rectangle has it as its content rectangle
 * again; the frame is never smaller than the content rectangle; a window's frame minus its
 * content view's frame gives the same decoration height as the class method. */
#include "poly_mak.h"

int test_main(void) {
    const NSWindowStyleMask masks[] = {NSWindowStyleMaskBorderless, NSWindowStyleMaskTitled, NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskResizable, NSWindowStyleMaskTitled | NSWindowStyleMaskFullSizeContentView, NSWindowStyleMaskResizable};
    const NSRect rects[] = {{{0, 0}, {10, 10}}, {{100, 200}, {300, 200}}, {{-50, -60}, {1, 1}}, {{3, 4}, {1000, 700}}};
    int ok = 1, bigger = 1;
    for (int i = 0; i < 5; i++)
        for (int j = 0; j < 4; j++) {
            NSRect f = [NSWindow frameRectForContentRect:rects[j] styleMask:masks[i]];
            NSRect back = [NSWindow contentRectForFrameRect:f styleMask:masks[i]];
            ok &= NSEqualRects(back, rects[j]);
            bigger &= f.size.width >= rects[j].size.width && f.size.height >= rects[j].size.height;
        }
    CK(ok);
    CK(bigger);
    NSWindow *w = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 123, 77));
    CKF(w.frame.size.height - w.contentView.frame.size.height, [NSWindow frameRectForContentRect:NSMakeRect(0, 0, 1, 1) styleMask:NSWindowStyleMaskTitled].size.height - 1);
    return finish();
}
