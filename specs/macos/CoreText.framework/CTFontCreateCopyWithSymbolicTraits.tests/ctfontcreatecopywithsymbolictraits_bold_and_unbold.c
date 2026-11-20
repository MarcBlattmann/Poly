/* poly-mac: coretext */
/* Asking the regular registered font for the bold trait gives PolyTest-Bold at the same size,
 * asking the bold one to drop it gives the regular; a copy with no change (trait 0, mask 0)
 * is the same font; a new size argument sets the size (0 keeps it); the copies have the
 * matching metrics; the family stays PolyTest. */
#include "poly_mct.h"

int test_main(void) {
    CK(pt_register());
    CTFontRef reg = CTFontCreateWithName(CFSTR("PolyTest-Regular"), 20, NULL);
    CTFontRef bold = CTFontCreateCopyWithSymbolicTraits(reg, 0, NULL, kCTFontTraitBold, kCTFontTraitBold);
    CK(pt_is(bold, "PolyTest-Bold"));
    CKF(CTFontGetSize(bold), 20);
    CK(CTFontGetSymbolicTraits(bold) == kCTFontTraitBold);
    CTFontRef back = CTFontCreateCopyWithSymbolicTraits(bold, 0, NULL, 0, kCTFontTraitBold);
    CK(pt_is(back, "PolyTest-Regular"));
    CK(pt_is(CTFontCreateCopyWithSymbolicTraits(reg, 0, NULL, 0, 0), "PolyTest-Regular"));
    CKF(CTFontGetSize(CTFontCreateCopyWithSymbolicTraits(reg, 50, NULL, 0, 0)), 50);
    CKF(CTFontGetSize(CTFontCreateCopyWithSymbolicTraits(reg, 0, NULL, 0, 0)), 20);
    CKF(CTFontGetAscent(CTFontCreateCopyWithSymbolicTraits(reg, 50, NULL, kCTFontTraitBold, kCTFontTraitBold)), 40);
    CK(cf_is(CTFontCopyFamilyName(bold), "PolyTest"));
    return finish();
}
