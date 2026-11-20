/* poly-mac: coretext */
/* The character set holds the cmap's code points: A, B, V, W, a, g, O, space, U+00E9,
 * U+0301 and the emoji U+1F600 (a long character); not 'z', U+0000, U+FFFF or U+10000;
 * it is a CFCharacterSet; both PolyTest fonts have the same set. */
#include "poly_mct.h"

int test_main(void) {
    CFCharacterSetRef cs = CTFontCopyCharacterSet(pt_font(0, 20));
    CK(cs != NULL && CFGetTypeID(cs) == CFCharacterSetGetTypeID());
    const UniChar yes[] = {'A', 'B', 'V', 'W', 'a', 'g', 'O', ' ', 0xE9, 0x301};
    for (int i = 0; i < 10; i++) CK(CFCharacterSetIsCharacterMember(cs, yes[i]));
    CK(CFCharacterSetIsLongCharacterMember(cs, 0x1F600));
    const UniChar no[] = {'z', 'b', 0, 0xFFFF, 0x4E2D};
    for (int i = 0; i < 5; i++) CK(!CFCharacterSetIsCharacterMember(cs, no[i]));
    CK(!CFCharacterSetIsLongCharacterMember(cs, 0x10000));
    CFCharacterSetRef cb = CTFontCopyCharacterSet(pt_font(1, 20));
    CK(CFCharacterSetIsCharacterMember(cb, 'A') && !CFCharacterSetIsCharacterMember(cb, 'z'));
    CK(CFEqual(cs, cb));
    return finish();
}
