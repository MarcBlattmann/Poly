/* poly-mac: coretext */
/* One glyph per character for plain text; a precomposed é (U+00E9) is one glyph with the
 * font's composite glyph 8; an 'a' followed by the combining mark U+0301 is two glyphs
 * (a, then the mark glyph 9) with the string range (0,2); an emoji (a surrogate pair) is
 * one glyph 10 with the range (0,2) and string index 0; the line's range always covers the
 * whole string. */
#include "poly_mct.h"

int test_main(void) {
    CTFontRef f = pt_font(0, 100);
    CTLineRef plain = pt_line("ABVa", f);
    CK(CTLineGetGlyphCount(plain) == 4);
    CTLineRef e = pt_line("\xc3\xa9", f);
    CK(CTLineGetGlyphCount(e) == 1);
    CGGlyph g[2];
    CTRunGetGlyphs(CFArrayGetValueAtIndex(CTLineGetGlyphRuns(e), 0), CFRangeMake(0, 0), g);
    CK(g[0] == 8);
    CTLineRef c = pt_line("a\xcc\x81", f);
    CK(CTLineGetGlyphCount(c) == 2);
    CFRange cr = CTLineGetStringRange(c);
    CK(cr.location == 0 && cr.length == 2);
    CTRunGetGlyphs(CFArrayGetValueAtIndex(CTLineGetGlyphRuns(c), 0), CFRangeMake(0, 0), g);
    CK(g[0] == 6 && g[1] == 9);
    CTLineRef em = pt_line("\xf0\x9f\x98\x80", f);
    CK(CTLineGetGlyphCount(em) == 1);
    CFRange er = CTLineGetStringRange(em);
    CK(er.location == 0 && er.length == 2);
    CFIndex idx[1];
    CTRunGetStringIndices(CFArrayGetValueAtIndex(CTLineGetGlyphRuns(em), 0), CFRangeMake(0, 0), idx);
    CK(idx[0] == 0);
    CTRunGetGlyphs(CFArrayGetValueAtIndex(CTLineGetGlyphRuns(em), 0), CFRangeMake(0, 0), g);
    CK(g[0] == 10);
    return finish();
}
