/* poly-mac: coretext
 * poly-mac: fonts DejaVuSans.ttf */
/* SYSTEM-DEPENDENT (Apple's font choices on this macOS). A Latin letter the PolyTest font
 * lacks falls back to 'Helvetica'; CJK U+4E2D to 'PingFangSC-Regular'; Arabic U+0627 to
 * 'GeezaPro'; the emoji U+1F601 to 'AppleColorEmoji'; for DejaVu Sans (which has Arabic and
 * U+1F601 but no CJK) U+4E2D also gives 'PingFangSC-Regular' and the Arabic letter stays in
 * DejaVu. */
#include "poly_mct.h"

static CTFontRef for_char(CTFontRef f, UniChar c) { return CTFontCreateForString(f, CFStringCreateWithCharacters(kCFAllocatorDefault, &c, 1), CFRangeMake(0, 1)); }

int test_main(void) {
    CTFontRef p = pt_font(0, 20);
    CK(pt_is(for_char(p, 'z'), "Helvetica"));
    CK(pt_is(for_char(p, 0x4E2D), "PingFangSC-Regular"));
    CK(pt_is(for_char(p, 0x0627), "GeezaPro"));
    UniChar emoji[] = {0xD83D, 0xDE01};
    CK(pt_is(CTFontCreateForString(p, CFStringCreateWithCharacters(kCFAllocatorDefault, emoji, 2), CFRangeMake(0, 2)), "AppleColorEmoji"));
    CTFontRef dj = pt_file_font("DejaVuSans.ttf", 20);
    CK(pt_is(for_char(dj, 0x4E2D), "PingFangSC-Regular"));
    CK(pt_is(for_char(dj, 0x0627), "DejaVuSans"));
    return finish();
}
