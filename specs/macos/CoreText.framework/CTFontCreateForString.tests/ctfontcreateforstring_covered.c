/* poly-mac: coretext */
/* When the font has every character of the range the result is that font (same PostScript
 * name, same size): for one character, for a range inside a longer string, for the
 * emoji pair; a range of 0 characters too; the original font is not changed. */
#include "poly_mct.h"

int test_main(void) {
    CTFontRef f = pt_font(0, 24);
    CFStringRef s = CFSTR("AVAW");
    CTFontRef r = CTFontCreateForString(f, s, CFRangeMake(0, 4));
    CK(r != NULL && pt_is(r, "PolyTest-Regular"));
    CKF(CTFontGetSize(r), 24);
    CK(pt_is(CTFontCreateForString(f, s, CFRangeMake(2, 1)), "PolyTest-Regular"));
    UniChar pair[] = {0xD83D, 0xDE00};
    CK(pt_is(CTFontCreateForString(f, CFStringCreateWithCharacters(kCFAllocatorDefault, pair, 2), CFRangeMake(0, 2)), "PolyTest-Regular"));
    CK(pt_is(CTFontCreateForString(f, s, CFRangeMake(1, 0)), "PolyTest-Regular"));
    CK(pt_is(CTFontCreateForString(pt_font(1, 24), s, CFRangeMake(0, 4)), "PolyTest-Bold"));
    CK(pt_is(f, "PolyTest-Regular"));
    return finish();
}
