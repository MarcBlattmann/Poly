/* poly-mac: appkit */
/* bitmapImageRepForCachingDisplayInRect: gives a bitmap for a view's area: for the view's whole bounds of 10 x 8 points
 * in a window on a display of scale 2 it is 20 x 16 pixels (SYSTEM-DEPENDENT: the scale of the display, printed) with its
 * size in points 10 x 8, 32 bits per pixel, 4 samples, with alpha, not planar, bytesPerRow 96 (a multiple of 32? here
 * exactly width * 4 rounded to 32 bytes: 80 -> 96, printed), and a colour space that is the display's profile, not
 * sRGB (printed as SYSTEM-DEPENDENT); for a sub-rectangle (1,1,4,2) it is 8 x 4 pixels; before the view is drawn into it, every byte of the
 * bitmap is 0; a view that is not in a window gets the main screen's scale (also 20 x 16) and the colour space "Generic RGB"
 * (printed, SYSTEM-DEPENDENT). */
#include "poly_mak.h"

int test_main(void) {
    NSView *v = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 10, 8)];
    NSBitmapImageRep *alone = [v bitmapImageRepForCachingDisplayInRect:v.bounds];
    double mainScale = NSScreen.mainScreen.backingScaleFactor;
    CK(alone.pixelsWide == (NSInteger)(10 * mainScale) && alone.pixelsHigh == (NSInteger)(8 * mainScale));
    value_ns("sys_alone_colorSpace", alone.colorSpace.localizedName);
    NSWindow *w = ak_window(NSWindowStyleMaskBorderless, NSMakeRect(0, 0, 10, 8));
    w.contentView = v;
    double scale = w.backingScaleFactor;
    NSBitmapImageRep *r = [v bitmapImageRepForCachingDisplayInRect:v.bounds];
    CK(r.pixelsWide == (NSInteger)(10 * scale) && r.pixelsHigh == (NSInteger)(8 * scale));
    CKF(r.size.width, 10);
    CKF(r.size.height, 8);
    CK(r.bitsPerPixel == 32 && r.samplesPerPixel == 4 && r.hasAlpha && !r.isPlanar && r.bitsPerSample == 8);
    CK(r.bytesPerRow >= r.pixelsWide * 4);
    CK(r.colorSpace != [NSColorSpace sRGBColorSpace]);
    int zero = 1;
    for (NSInteger i = 0; i < r.bytesPerRow * r.pixelsHigh; i++) zero &= r.bitmapData[i] == 0;
    CK(zero);
    NSBitmapImageRep *s = [v bitmapImageRepForCachingDisplayInRect:NSMakeRect(1, 1, 4, 2)];
    CK(s.pixelsWide == (NSInteger)(4 * scale) && s.pixelsHigh == (NSInteger)(2 * scale));
    value("sys_rep_pixelsWide", r.pixelsWide);
    value("sys_rep_pixelsHigh", r.pixelsHigh);
    value("sys_rep_bytesPerRow", r.bytesPerRow);
    value_ns("sys_rep_colorSpace", r.colorSpace.localizedName);
    [w close];
    return finish();
}
