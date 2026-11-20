/* poly-mac: appkit */
/* +contentRectForFrameRect:styleMask: removes the decoration from a frame: borderless nothing; titled
 * masks take 32 off the height (a 300 x 232 frame has a 300 x 200 content rectangle) at the same origin;
 * with the full-size content view flag the content rectangle is the frame plus the title bar's
 * 32 (the frame does not include the title bar twice: 232 for 200); the panel masks: utility 208 for
 * 232, HUD 213 for 232. */
#include "poly_mak.h"

int test_main(void) {
    NSRect f = NSMakeRect(100, 200, 300, 232);
    CK(NSEqualRects([NSWindow contentRectForFrameRect:f styleMask:NSWindowStyleMaskBorderless], NSMakeRect(100, 200, 300, 232)));
    CK(NSEqualRects([NSWindow contentRectForFrameRect:f styleMask:NSWindowStyleMaskTitled], NSMakeRect(100, 200, 300, 200)));
    CK(NSEqualRects([NSWindow contentRectForFrameRect:f styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable], NSMakeRect(100, 200, 300, 200)));
    CK(NSEqualRects([NSWindow contentRectForFrameRect:f styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskFullSizeContentView], NSMakeRect(100, 200, 300, 232)));
    CK(NSEqualRects([NSWindow contentRectForFrameRect:f styleMask:NSWindowStyleMaskResizable], NSMakeRect(100, 200, 300, 232)));
    CK(NSEqualRects([NSWindow contentRectForFrameRect:f styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskUtilityWindow], NSMakeRect(100, 200, 300, 208)));
    CK(NSEqualRects([NSWindow contentRectForFrameRect:f styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskHUDWindow], NSMakeRect(100, 200, 300, 213)));
    return finish();
}
