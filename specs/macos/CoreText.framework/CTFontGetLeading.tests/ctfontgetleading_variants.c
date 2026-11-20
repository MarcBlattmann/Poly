/* poly-mac: coretext */
/* CTFontGetLeading gives the same for the bold font as the regular one (the same tables), follows
 * a copy at another size (CTFontGetLeading(copy at 50) is 5), follows the
 * vertical scale of a font matrix (a scale of 2 in x and 3 in y makes it 3 times as
 * large, the x scale is not used) and is the same through CTFontCreateWithName for a
 * registered font. */
#include "poly_mct.h"

int test_main(void) {
    CKF(CTFontGetLeading(pt_font(1, 20)), 2.0000);
    CKF(CTFontGetLeading(pt_font(1, 20)), CTFontGetLeading(pt_font(0, 20)));
    CKF(CTFontGetLeading(CTFontCreateCopyWithAttributes(pt_font(0, 10), 50, NULL, NULL)), 5.0000);
    CGAffineTransform t = CGAffineTransformMake(2, 0, 0, 3, 0, 0);
    CKF(CTFontGetLeading(CTFontCreateWithFontDescriptor(pt_descriptor(0), 20, &t)), 6.0000);
    CK(pt_register());
    CKF(CTFontGetLeading(CTFontCreateWithName(CFSTR("PolyTest-Regular"), 40, NULL)), 4.0000);
    CKF(CTFontGetLeading(CTFontCreateWithName(CFSTR("PolyTest-Bold"), 40, NULL)), 4.0000);
    return finish();
}
