/* poly-mac: corefoundation */
/* A UTF-8 byte order mark at the start of the C string is dropped; one in the
 * middle is kept as the character U+FEFF. */
#include "poly_mcf.h"

#define U8(s) CFStringCreateWithCString(kCFAllocatorDefault, s, kCFStringEncodingUTF8)

int test_main(void) {
    CK(UNITS(U8("\xef\xbb\xbf" "a"), 0x61));
    CFStringRef only = U8("\xef\xbb\xbf");
    CK(only != NULL && CFStringGetLength(only) == 0);
    CK(UNITS(U8("a\xef\xbb\xbf" "b"), 0x61, 0xfeff, 0x62));
    return finish();
}
