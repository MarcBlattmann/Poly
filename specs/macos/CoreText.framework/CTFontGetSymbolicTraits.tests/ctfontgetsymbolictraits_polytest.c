/* poly-mac: coretext */
/* The symbolic traits come from the font's style bits: the regular font has none (0), the bold
 * font has kCTFontTraitBold (1<<1 = 2); neither is italic, monospace or vertical;
 * the traits do not depend on size or matrix. */
#include "poly_mct.h"

int test_main(void) {
    CK(kCTFontTraitItalic == 1 && kCTFontTraitBold == 2 && kCTFontTraitMonoSpace == (1 << 10));
    CK(CTFontGetSymbolicTraits(pt_font(0, 20)) == 0);
    CK(CTFontGetSymbolicTraits(pt_font(1, 20)) == kCTFontTraitBold);
    CK(CTFontGetSymbolicTraits(pt_font(1, 5)) == kCTFontTraitBold);
    CGAffineTransform t = CGAffineTransformMakeScale(2, 1);
    CK(CTFontGetSymbolicTraits(CTFontCreateWithFontDescriptor(pt_descriptor(1), 20, &t)) == kCTFontTraitBold);
    CK((CTFontGetSymbolicTraits(pt_font(1, 20)) & (kCTFontTraitItalic | kCTFontTraitMonoSpace | kCTFontTraitVertical)) == 0);
    return finish();
}
