/* poly-mac: coretext */
/* CTFontCopyAvailableTables lists the tags of the font file's tables as a CFArray of
 * numbers holding the four characters (big endian): OS/2, cmap, glyf, head, hhea,
 * hmtx, kern, loca, maxp, name and post, in the file's (sorted) order, 11 of them;
 * every listed tag can be copied and every copy is non-empty; the same list for the
 * bold font. */
#include "poly_mct.h"

static void tag(char *o, CTFontTableTag t) { o[0] = (char)(t >> 24); o[1] = (char)(t >> 16); o[2] = (char)(t >> 8); o[3] = (char)t; o[4] = 0; }

int test_main(void) {
    CTFontRef f = pt_font(0, 20);
    CFArrayRef tags = CTFontCopyAvailableTables(f, kCTFontTableOptionNoOptions);
    CK(tags != NULL && CFArrayGetCount(tags) == 11);
    const char *want[11] = {"OS/2", "cmap", "glyf", "head", "hhea", "hmtx", "kern", "loca", "maxp", "name", "post"};
    for (CFIndex i = 0; i < 11; i++) {
        char t[5];
        tag(t, (CTFontTableTag)(uintptr_t)CFArrayGetValueAtIndex(tags, i));
        CK(str_eq(t, want[i]));
        CFDataRef d = CTFontCopyTable(f, (CTFontTableTag)(uintptr_t)CFArrayGetValueAtIndex(tags, i), 0);
        CK(d != NULL && CFDataGetLength(d) > 0);
    }
    CFArrayRef bt = CTFontCopyAvailableTables(pt_font(1, 20), 0);
    CK(bt != NULL && CFArrayGetCount(bt) == 11);
    return finish();
}
