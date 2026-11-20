/* poly-mac: coretext
 * poly-mac: fonts DejaVuSans.ttf */
/* SYSTEM-DEPENDENT. The fallback run for 'z' in a PolyTest line is Helvetica (ascent 77, descent 23
 * at size 100, glyph 93, advance 50); for the CJK character U+4E2D in a DejaVu Sans line it is
 * PingFangSC-Regular; the line's ascent is the largest of the runs (PolyTest 80). */
#include "poly_mct.h"

int test_main(void) {
    CTLineRef l = pt_line("AzA", pt_font(0, 100));
    CTRunRef r = CFArrayGetValueAtIndex(CTLineGetGlyphRuns(l), 1);
    CTFontRef fb = (CTFontRef)CFDictionaryGetValue(CTRunGetAttributes(r), kCTFontAttributeName);
    CK(pt_is(fb, "Helvetica"));
    CGFloat a, d, le;
    CKF(CTRunGetTypographicBounds(r, CFRangeMake(0, 0), &a, &d, &le), 50);
    CKF(a, 77);
    CKF(d, 23);
    CGGlyph g;
    CTRunGetGlyphs(r, CFRangeMake(0, 1), &g);
    CK(g == 93);
    CGFloat la, ld;
    CTLineGetTypographicBounds(l, &la, &ld, NULL);
    CKF(la, 80);
    CKF(ld, 23);
    CTLineRef c = pt_line("a\xe4\xb8\xad", pt_file_font("DejaVuSans.ttf", 20));
    CFArrayRef runs = CTLineGetGlyphRuns(c);
    CK(CFArrayGetCount(runs) == 2);
    CK(pt_is((CTFontRef)CFDictionaryGetValue(CTRunGetAttributes(CFArrayGetValueAtIndex(runs, 1)), kCTFontAttributeName), "PingFangSC-Regular"));
    return finish();
}
