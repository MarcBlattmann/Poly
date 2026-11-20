/* poly-mac: coretext */
/* Horizontal advances scale with the size: at 100 A 60, B 50, V 70, W 140, a 50, space 30, the
 * accent mark 0, .notdef 50; the return value is their sum (kerning is not applied
 * between glyphs); at size 10 a tenth of that; a NULL advances array only gives the
 * sum; a count of 0 gives 0. */
#include "poly_mct.h"

int test_main(void) {
    CTFontRef f = pt_font(0, 100);
    CGGlyph g[] = {2, 3, 4, 5, 6, 1, 9, 0};
    CGSize adv[8];
    double total = CTFontGetAdvancesForGlyphs(f, kCTFontOrientationHorizontal, g, adv, 8);
    const double want[8] = {60, 50, 70, 140, 50, 30, 0, 50};
    double sum = 0;
    for (int i = 0; i < 8; i++) {
        CKF(adv[i].width, want[i]);
        CKF(adv[i].height, 0);
        sum += want[i];
    }
    CKF(total, sum);
    CGSize a10[2];
    CKF(CTFontGetAdvancesForGlyphs(pt_font(0, 10), kCTFontOrientationHorizontal, g, a10, 2), 6 + 5);
    CKF(a10[0].width, 6);
    CKF(CTFontGetAdvancesForGlyphs(f, kCTFontOrientationHorizontal, g, NULL, 4), 60 + 50 + 70 + 140);
    CGGlyph av[2] = {2, 4};
    CKF(CTFontGetAdvancesForGlyphs(f, kCTFontOrientationHorizontal, av, NULL, 2), 130);
    CKF(CTFontGetAdvancesForGlyphs(f, kCTFontOrientationHorizontal, g, adv, 0), 0);
    return finish();
}
