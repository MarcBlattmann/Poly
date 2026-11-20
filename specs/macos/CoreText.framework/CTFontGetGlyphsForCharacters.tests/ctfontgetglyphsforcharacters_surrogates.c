/* poly-mac: coretext */
/* A character above U+FFFF is given as two UTF-16 units: the glyph is stored in the slot
 * of the high surrogate, the slot of the low one gets 0 and the call returns true;
 * two such characters, a lone high or low surrogate (no glyph: false) and a pair for
 * a character the font lacks (U+1F601: false, glyph 0) behave likewise. */
#include "poly_mct.h"

int test_main(void) {
    CTFontRef f = pt_font(0, 20);
    UniChar pair[] = {0xD83D, 0xDE00};
    CGGlyph g[2] = {99, 99};
    CK(CTFontGetGlyphsForCharacters(f, pair, g, 2));
    CK(g[0] == 10 && g[1] == 0);
    UniChar mix[] = {'A', 0xD83D, 0xDE00, 'B'};
    CGGlyph m[4] = {99, 99, 99, 99};
    CK(CTFontGetGlyphsForCharacters(f, mix, m, 4));
    CK(m[0] == 2 && m[1] == 10 && m[2] == 0 && m[3] == 3);
    UniChar missing[] = {0xD83D, 0xDE01};
    CGGlyph x[2] = {99, 99};
    CK(!CTFontGetGlyphsForCharacters(f, missing, x, 2));
    CK(x[0] == 0);
    UniChar lone[] = {0xD83D};
    CGGlyph l[1] = {99};
    CK(!CTFontGetGlyphsForCharacters(f, lone, l, 1));
    CK(l[0] == 0);
    UniChar lowlone[] = {0xDE00};
    CK(!CTFontGetGlyphsForCharacters(f, lowlone, l, 1));
    return finish();
}
