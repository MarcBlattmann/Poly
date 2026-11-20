/* poly-mac: coretext */
/* Each span with its own font attribute is its own run: 'AB' with the regular font on A and the
 * bold font on B gives 2 runs (string ranges (0,1) and (1,1)), each with its font
 * in the run attributes; the bold B is 56 wide at size 100, so the line is 60 + 56; one
 * font over the whole string gives one run. */
#include "poly_mct.h"

int test_main(void) {
    CTFontRef reg = pt_font(0, 100), bold = pt_font(1, 100);
    CFMutableAttributedStringRef as = CFAttributedStringCreateMutable(kCFAllocatorDefault, 0);
    CFAttributedStringReplaceString(as, CFRangeMake(0, 0), CFSTR("AB"));
    CFAttributedStringSetAttribute(as, CFRangeMake(0, 1), kCTFontAttributeName, reg);
    CFAttributedStringSetAttribute(as, CFRangeMake(1, 1), kCTFontAttributeName, bold);
    CTLineRef l = CTLineCreateWithAttributedString(as);
    CFArrayRef runs = CTLineGetGlyphRuns(l);
    CK(CFArrayGetCount(runs) == 2);
    CTRunRef r0 = CFArrayGetValueAtIndex(runs, 0), r1 = CFArrayGetValueAtIndex(runs, 1);
    CFRange a = CTRunGetStringRange(r0), b = CTRunGetStringRange(r1);
    CK(a.location == 0 && a.length == 1 && b.location == 1 && b.length == 1);
    CK(pt_is((CTFontRef)CFDictionaryGetValue(CTRunGetAttributes(r0), kCTFontAttributeName), "PolyTest-Regular"));
    CK(pt_is((CTFontRef)CFDictionaryGetValue(CTRunGetAttributes(r1), kCTFontAttributeName), "PolyTest-Bold"));
    CKF(CTLineGetTypographicBounds(l, NULL, NULL, NULL), 116);
    CK(CFArrayGetCount(CTLineGetGlyphRuns(pt_line("AB", reg))) == 1);
    return finish();
}
