/* poly-mac: coretext */
/* The set does not depend on size, matrix or copy; a fallback font made for a missing
 * character has a set that contains it and is not the PolyTest set; a set taken twice is
 * equal. */
#include "poly_mct.h"

int test_main(void) {
    CFCharacterSetRef a = CTFontCopyCharacterSet(pt_font(0, 10)), b = CTFontCopyCharacterSet(pt_font(0, 500));
    CK(CFEqual(a, b));
    CGAffineTransform t = CGAffineTransformMakeScale(3, 1);
    CK(CFEqual(a, CTFontCopyCharacterSet(CTFontCreateWithFontDescriptor(pt_descriptor(0), 20, &t))));
    CK(CFEqual(a, CTFontCopyCharacterSet(CTFontCreateCopyWithAttributes(pt_font(0, 10), 77, NULL, NULL))));
    UniChar cjk[] = {0x4E2D};
    CTFontRef fb = CTFontCreateForString(pt_font(0, 20), CFStringCreateWithCharacters(kCFAllocatorDefault, cjk, 1), CFRangeMake(0, 1));
    CK(fb != NULL);
    CFCharacterSetRef fs = CTFontCopyCharacterSet(fb);
    CK(CFCharacterSetIsCharacterMember(fs, 0x4E2D));
    CK(!CFEqual(fs, a));
    return finish();
}
