/* poly-mac: appkit */
/* The same button in the DARK appearance over an opaque (30,30,30) background (SYSTEM-DEPENDENT, printed as sys_*): the corner is
 * the background 0x1e1e1e, the body is 0x303030 (the translucent bezel is light here: 30 -> 48), the label is light
 * (0xdfdfdf at its lightest). What is fixed: opaque, body lighter than the background, label lighter than the body, rounded corners
 * showing the background, and a different picture from the light one. */
#include "poly_mak.h"

static NSBitmapImageRep *draw(NSAppearance *ap, double bg) {
    NSButton *b = [NSButton buttonWithTitle:@"OK" target:nil action:NULL];
    b.frame = NSMakeRect(0, 0, 60, 24);
    return ak_render_on(b, ap, 2, [NSColor colorWithSRGBRed:bg green:bg blue:bg alpha:1]);
}

int test_main(void) {
    NSBitmapImageRep *r = draw(ak_dark(), 30 / 255.0);
    CK(r.pixelsWide == 120 && r.pixelsHigh == 48);
    int opaque = 1;
    for (int y = 0; y < 48; y++)
        for (int x = 0; x < 120; x++) opaque &= (ak_px(r, x, y) & 255) == 255;
    CK(opaque);
    CKV(ak_px(r, 0, 0) == 0x1e1e1eff, ak_px(r, 0, 0));
    unsigned body = ak_px(r, 20, 24);
    CK(((body >> 24) & 255) > 30 && ((body >> 24) & 255) < 90);
    int lightest = 0;
    for (int x = 40; x < 80; x++) {
        int g = (int)(ak_px(r, x, 24) >> 24);
        if (g > lightest) lightest = g;
    }
    CK(lightest > (int)(body >> 24) + 80);
    NSBitmapImageRep *l = draw(ak_light(), 236 / 255.0);
    CK(ak_hash(l) != ak_hash(r));
    CK(ak_hash(r) == ak_hash(draw(ak_dark(), 30 / 255.0)));
    value("sys_body", ak_px(r, 20, 24));
    value("sys_lightest_label_gray", lightest);
    ak_dump("sys_top", r, 0, 0, 10, 1);
    ak_dump("sys_row2", r, 0, 2, 10, 3);
    ak_dump("sys_mid_left", r, 0, 24, 10, 25);
    ak_dump("sys_mid_label", r, 50, 24, 62, 25);
    value("sys_hash", (long)(ak_hash(r) & 0x7fffffff));
    return finish();
}
