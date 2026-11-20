/* poly-mac: appkit */
/* A push button "OK" of 60 x 24 points drawn (displayRectIgnoringOpacity:inContext:) at 2 pixels per point into an sRGB bitmap over
 * an opaque (236,236,236) background in the LIGHT appearance: the output is 120 x 48 pixels; ALL colours here are
 * SYSTEM-DEPENDENT (this macOS; printed as sys_*): the corner pixel (0,0) is the background 0xececec, the body inside the bezel is
 * 0xdadada (the bezel is a translucent dark layer over the background: 236 -> 218), the first pixels of the top row fade in
 * from the corner (0xeaeaea, 0xe5e5e5, 0xe1e1e1 from x 7), the label "OK" is dark (0x212121 at its darkest) in the middle
 * row, everything is opaque (alpha 0xff). What is fixed: the bitmap is opaque, the body is darker than the
 * background, the label is darker than the body, the corners are the background (rounded), the picture is symmetric left-right
 * for the bezel (row 24, x 0..7 equals the mirror at x 119..112) and the same view drawn twice gives the same bytes. The hash of the
 * bitmap differs between x86_64 and arm64 by small rounding steps (recorded per architecture). */
#include "poly_mak.h"

static NSBitmapImageRep *draw(void) {
    NSButton *b = [NSButton buttonWithTitle:@"OK" target:nil action:NULL];
    b.frame = NSMakeRect(0, 0, 60, 24);
    return ak_render_on(b, ak_light(), 2, [NSColor colorWithSRGBRed:236 / 255.0 green:236 / 255.0 blue:236 / 255.0 alpha:1]);
}

int test_main(void) {
    NSBitmapImageRep *r = draw();
    CK(r.pixelsWide == 120 && r.pixelsHigh == 48);
    int opaque = 1;
    for (int y = 0; y < 48; y++)
        for (int x = 0; x < 120; x++) opaque &= (ak_px(r, x, y) & 255) == 255;
    CK(opaque);
    CKV(ak_px(r, 0, 0) == 0xecececff, ak_px(r, 0, 0));
    unsigned body = ak_px(r, 20, 24);
    CK(((body >> 24) & 255) < 236 && ((body >> 24) & 255) > 200);
    CK(ak_px(r, 20, 24) == ak_px(r, 100, 24) && ak_px(r, 20, 10) == body);
    int darkest = 255;
    for (int x = 40; x < 80; x++) {
        int g = (int)(ak_px(r, x, 24) >> 24);
        if (g < darkest) darkest = g;
    }
    CK(darkest < (int)(body >> 24) - 80);
    int mirror = 1;
    for (int x = 0; x < 8; x++) mirror &= ak_px(r, x, 24) == ak_px(r, 119 - x, 24);
    CK(mirror);
    CK(ak_hash(r) == ak_hash(draw()));
    value("sys_body", ak_px(r, 20, 24));
    value("sys_darkest_label_gray", darkest);
    ak_dump("sys_top", r, 0, 0, 10, 1);
    ak_dump("sys_row2", r, 0, 2, 10, 3);
    ak_dump("sys_mid_left", r, 0, 24, 10, 25);
    ak_dump("sys_mid_label", r, 50, 24, 62, 25);
    value("sys_hash", (long)(ak_hash(r) & 0x7fffffff));
    return finish();
}
