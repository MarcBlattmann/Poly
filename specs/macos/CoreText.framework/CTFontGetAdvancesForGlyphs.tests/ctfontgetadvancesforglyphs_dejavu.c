/* poly-mac: coretext
 * poly-mac: fonts DejaVuSans.ttf */
/* DejaVu Sans advances are the font's design units scaled and not rounded to pixels: at size
 * 100 A 68.41, V 68.41, a 61.28, space 31.79, W 98.88; at size 12 the A is 8.21; the sum is
 * returned and no kerning enters it. */
#include "poly_mct.h"

int test_main(void) {
    CTFontRef f = pt_file_font("DejaVuSans.ttf", 100);
    UniChar chs[] = {'A', 'V', 'a', ' ', 'W'};
    CGGlyph g[5];
    CK(CTFontGetGlyphsForCharacters(f, chs, g, 5));
    CGSize adv[5];
    double total = CTFontGetAdvancesForGlyphs(f, kCTFontOrientationHorizontal, g, adv, 5);
    const double want[5] = {68.41, 68.41, 61.28, 31.79, 98.88};
    double sum = 0;
    for (int i = 0; i < 5; i++) {
        CKF(adv[i].width, want[i]);
        sum += adv[i].width;
    }
    CKF(total, sum);
    CTFontRef f12 = pt_file_font("DejaVuSans.ttf", 12);
    CGGlyph a;
    UniChar ca = 'A';
    CTFontGetGlyphsForCharacters(f12, &ca, &a, 1);
    CGSize a12;
    CTFontGetAdvancesForGlyphs(f12, kCTFontOrientationHorizontal, &a, &a12, 1);
    CKF(a12.width, 8.21);
    return finish();
}
