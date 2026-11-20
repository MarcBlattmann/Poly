/* poly-mac: coretext */
/* A font made from a data descriptor at a size has that size (0 gives 12, a negative
 * size too); it has the data's metrics and names; two fonts of one descriptor and
 * size are CFEqual, of different size or another font not; the font has CTFontGetTypeID. */
#include "poly_mct.h"

int test_main(void) {
    CTFontDescriptorRef d = pt_descriptor(0);
    CTFontRef f = CTFontCreateWithFontDescriptor(d, 20, NULL);
    CK(f != NULL && CFGetTypeID(f) == CTFontGetTypeID());
    CKF(CTFontGetSize(f), 20);
    CKF(CTFontGetAscent(f), 16);
    CK(pt_is(f, "PolyTest-Regular"));
    CKF(CTFontGetSize(CTFontCreateWithFontDescriptor(d, 0, NULL)), 12);
    CKF(CTFontGetSize(CTFontCreateWithFontDescriptor(d, -3, NULL)), 12);
    CK(CFEqual(f, CTFontCreateWithFontDescriptor(d, 20, NULL)));
    CK(!CFEqual(f, CTFontCreateWithFontDescriptor(d, 21, NULL)));
    CK(!CFEqual(f, CTFontCreateWithFontDescriptor(pt_descriptor(1), 20, NULL)));
    return finish();
}
