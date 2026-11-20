/* poly-mac: coretext */
/* A font with a size of 0 (the default 12) and the identity or a rotation matrix keeps
 * 1000; the value does not depend on the font's own style: the bold font made by
 * CTFontCreateCopyWithSymbolicTraits has 1000 too; the value is the same in two
 * fonts made from two descriptors of the same data. */
#include "poly_mct.h"

int test_main(void) {
    CGAffineTransform rot = CGAffineTransformMakeRotation(0.5);
    CK(CTFontGetUnitsPerEm(CTFontCreateWithFontDescriptor(pt_descriptor(0), 0, &rot)) == 1000);
    CTFontRef r = pt_font(0, 30);
    CK(CTFontGetUnitsPerEm(CTFontCreateCopyWithSymbolicTraits(r, 0, NULL, 0, 0)) == 1000);
    CK(CTFontGetUnitsPerEm(pt_font(0, 30)) == CTFontGetUnitsPerEm(pt_font(0, 31)));
    CK(CTFontGetUnitsPerEm(pt_font(1, 12)) == CTFontGetUnitsPerEm(pt_font(0, 12)));
    CK(CTFontGetUnitsPerEm(pt_font(0, 12)) > 0);
    return finish();
}
