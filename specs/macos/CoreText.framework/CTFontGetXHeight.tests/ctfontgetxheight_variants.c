/* poly-mac: coretext */
/* CTFontGetXHeight gives the same for the bold font as the regular one (the same tables), follows
 * a copy at another size (CTFontGetXHeight(copy at 50) is 25), follows the
 * vertical scale of a font matrix (a scale of 2 in x and 3 in y makes it 3 times as
 * large, the x scale is not used) and is the same through CTFontCreateWithName for a
 * registered font. */
#include "poly_mct.h"

int test_main(void) {
    CKF(CTFontGetXHeight(pt_font(1, 20)), 10.0000);
    CKF(CTFontGetXHeight(pt_font(1, 20)), CTFontGetXHeight(pt_font(0, 20)));
    CKF(CTFontGetXHeight(CTFontCreateCopyWithAttributes(pt_font(0, 10), 50, NULL, NULL)), 25.0000);
    CGAffineTransform t = CGAffineTransformMake(2, 0, 0, 3, 0, 0);
    CKF(CTFontGetXHeight(CTFontCreateWithFontDescriptor(pt_descriptor(0), 20, &t)), 30.0000);
    CK(pt_register());
    CKF(CTFontGetXHeight(CTFontCreateWithName(CFSTR("PolyTest-Regular"), 40, NULL)), 20.0000);
    CKF(CTFontGetXHeight(CTFontCreateWithName(CFSTR("PolyTest-Bold"), 40, NULL)), 20.0000);
    return finish();
}
