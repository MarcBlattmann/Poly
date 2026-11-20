/* poly-mac: coretext */
/* CTFontGetCapHeight gives the same for the bold font as the regular one (the same tables), follows
 * a copy at another size (CTFontGetCapHeight(copy at 50) is 35), follows the
 * vertical scale of a font matrix (a scale of 2 in x and 3 in y makes it 3 times as
 * large, the x scale is not used) and is the same through CTFontCreateWithName for a
 * registered font. */
#include "poly_mct.h"

int test_main(void) {
    CKF(CTFontGetCapHeight(pt_font(1, 20)), 14.0000);
    CKF(CTFontGetCapHeight(pt_font(1, 20)), CTFontGetCapHeight(pt_font(0, 20)));
    CKF(CTFontGetCapHeight(CTFontCreateCopyWithAttributes(pt_font(0, 10), 50, NULL, NULL)), 35.0000);
    CGAffineTransform t = CGAffineTransformMake(2, 0, 0, 3, 0, 0);
    CKF(CTFontGetCapHeight(CTFontCreateWithFontDescriptor(pt_descriptor(0), 20, &t)), 42.0000);
    CK(pt_register());
    CKF(CTFontGetCapHeight(CTFontCreateWithName(CFSTR("PolyTest-Regular"), 40, NULL)), 28.0000);
    CKF(CTFontGetCapHeight(CTFontCreateWithName(CFSTR("PolyTest-Bold"), 40, NULL)), 28.0000);
    return finish();
}
