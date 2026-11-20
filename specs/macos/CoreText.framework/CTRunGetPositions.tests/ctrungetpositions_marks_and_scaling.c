/* poly-mac: coretext */
/* Positions scale with the size (size 10: B at x 6); a combining mark after 'a' is placed by
 * CoreText itself since the font has no mark positioning: the 'a' advance becomes 20 and the mark
 * (advance 30) sits at (20, 60) at size 100, so the pair is as wide as before (50); the pair's
 * run status is 16; a plain run's status is 0; the pointer accessors give the same
 * values as the copying ones, or NULL; the text matrix of a run is the identity. */
#include "poly_mct.h"

int test_main(void) {
    CTRunRef s = CFArrayGetValueAtIndex(CTLineGetGlyphRuns(pt_line("AB", pt_font(0, 10))), 0);
    CGPoint sp[2];
    CTRunGetPositions(s, CFRangeMake(0, 0), sp);
    CKF(sp[1].x, 6);
    CTRunRef m = CFArrayGetValueAtIndex(CTLineGetGlyphRuns(pt_line("a\xcc\x81", pt_font(0, 100))), 0);
    CGPoint mp[2];
    CGSize ma[2];
    CTRunGetPositions(m, CFRangeMake(0, 0), mp);
    CTRunGetAdvances(m, CFRangeMake(0, 0), ma);
    CKF(mp[0].x, 0); CKF(mp[1].x, 20); CKF(mp[1].y, 60);
    CKF(ma[0].width, 20); CKF(ma[1].width, 30);
    CK(CTRunGetStatus(m) == 16);
    CK(CTRunGetStatus(s) == 0);
    const CGPoint *ptr = CTRunGetPositionsPtr(s);
    CK(ptr == NULL || (near(ptr[1].x, 6)));
    const CGGlyph *gp = CTRunGetGlyphsPtr(s);
    CK(gp == NULL || (gp[0] == 2 && gp[1] == 3));
    CGAffineTransform tm = CTRunGetTextMatrix(s);
    CK(tm.a == 1 && tm.d == 1 && tm.b == 0 && tm.c == 0);
    return finish();
}
