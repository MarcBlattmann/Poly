/* poly-mac: coretext */
/* The glyph ids of the PolyTest font are those of its cmap: A 2, B 3, V 4, W 5, a 6, g 7,
 * U+00E9 8, U+0301 9, space 1, O 11. The call returns true when every character has a
 * glyph; for a character the font lacks ('z') it returns false, stores glyph 0 for
 * it and still stores the others; a count of 0 returns true; the same through a font at
 * any size. */
#include "poly_mct.h"

int test_main(void) {
    CTFontRef f = pt_font(0, 33);
    UniChar all[] = {'A', 'B', 'V', 'W', 'a', 'g', 0xE9, 0x301, ' ', 'O'};
    CGGlyph g[10];
    CK(CTFontGetGlyphsForCharacters(f, all, g, 10));
    const CGGlyph want[10] = {2, 3, 4, 5, 6, 7, 8, 9, 1, 11};
    for (int i = 0; i < 10; i++) CK(g[i] == want[i]);
    UniChar mixed[] = {'A', 'z', 'B'};
    CGGlyph m[3] = {99, 99, 99};
    CK(!CTFontGetGlyphsForCharacters(f, mixed, m, 3));
    CK(m[0] == 2 && m[1] == 0 && m[2] == 3);
    CGGlyph none[1] = {77};
    CK(CTFontGetGlyphsForCharacters(f, all, none, 0));
    CK(none[0] == 77);
    CGGlyph b[1];
    UniChar a = 'A';
    CK(CTFontGetGlyphsForCharacters(pt_font(1, 7), &a, b, 1) && b[0] == 2);
    return finish();
}
