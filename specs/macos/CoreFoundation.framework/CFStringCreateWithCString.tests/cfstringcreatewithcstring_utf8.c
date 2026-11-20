/* poly-mac: corefoundation */
/* A UTF-8 C string becomes a string of UTF-16 code units: ASCII one for one,
 * accents one unit, characters above U+FFFF a surrogate pair. The results are
 * read with CFStringGetLength and CFStringGetCharacterAtIndex. */
#include "poly_mcf.h"

#define U8(s) CFStringCreateWithCString(kCFAllocatorDefault, s, kCFStringEncodingUTF8)

int test_main(void) {
    CK(UNITS(U8("abc"), 0x61, 0x62, 0x63));
    CK(UNITS(U8("h\xc3\xa9llo"), 0x68, 0xe9, 0x6c, 0x6c, 0x6f));
    CK(UNITS(U8("\xe2\x82\xac"), 0x20ac));
    CK(UNITS(U8("\xf0\x9f\x98\x80"), 0xd83d, 0xde00));
    CK(UNITS(U8("a\xf0\x9f\x98\x80z"), 0x61, 0xd83d, 0xde00, 0x7a));
    CK(CFStringGetLength(U8("")) == 0);
    CK(CFGetTypeID(U8("x")) == CFStringGetTypeID());
    return finish();
}
