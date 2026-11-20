/* poly-mac: coretext */
/* A line in one font is one run holding all its glyphs; the run's glyph count, string range,
 * font attribute and typographic bounds (width 110, ascent 80, descent 20, leading 10 at
 * size 100) are the line's; two fonts make two runs in string order, each with its glyph
 * count; the runs array is a CFArray of CTRun objects. */
#include "poly_mct.h"

int test_main(void) {
    CTFontRef f = pt_font(0, 100);
    CFArrayRef runs = CTLineGetGlyphRuns(pt_line("AB", f));
    CK(CFArrayGetCount(runs) == 1);
    CTRunRef r = CFArrayGetValueAtIndex(runs, 0);
    CK(CFGetTypeID(r) == CTRunGetTypeID());
    CK(CTRunGetGlyphCount(r) == 2);
    CFRange sr = CTRunGetStringRange(r);
    CK(sr.location == 0 && sr.length == 2);
    CK(pt_is((CTFontRef)CFDictionaryGetValue(CTRunGetAttributes(r), kCTFontAttributeName), "PolyTest-Regular"));
    CGFloat a, d, l;
    CKF(CTRunGetTypographicBounds(r, CFRangeMake(0, 0), &a, &d, &l), 110);
    CKF(a, 80);
    CKF(d, 20);
    CKF(l, 10);
    CFMutableAttributedStringRef as = CFAttributedStringCreateMutable(kCFAllocatorDefault, 0);
    CFAttributedStringReplaceString(as, CFRangeMake(0, 0), CFSTR("ABB"));
    CFAttributedStringSetAttribute(as, CFRangeMake(0, 1), kCTFontAttributeName, f);
    CFAttributedStringSetAttribute(as, CFRangeMake(1, 2), kCTFontAttributeName, pt_font(1, 100));
    CFArrayRef two = CTLineGetGlyphRuns(CTLineCreateWithAttributedString(as));
    CK(CFArrayGetCount(two) == 2);
    CK(CTRunGetGlyphCount(CFArrayGetValueAtIndex(two, 0)) == 1 && CTRunGetGlyphCount(CFArrayGetValueAtIndex(two, 1)) == 2);
    CK(CTRunGetStringRange(CFArrayGetValueAtIndex(two, 1)).location == 1);
    return finish();
}
