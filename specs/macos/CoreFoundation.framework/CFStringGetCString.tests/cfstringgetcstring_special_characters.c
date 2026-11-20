/* poly-mac: corefoundation */
/* Characters above U+FFFF come out as one four-byte UTF-8 sequence; an
 * embedded U+0000 is written as a NUL byte; a lone surrogate cannot be
 * converted. */
#include "poly_mcf.h"

int test_main(void) {
    char b[16];
    memset(b, 0x55, sizeof b);
    CK(CFStringGetCString(S("a\xf0\x9f\x98\x80"), b, 10, kCFStringEncodingUTF8));
    CK(mem_eq(b, "a\xf0\x9f\x98\x80\0", 6) && (unsigned char)b[6] == 0x55);
    UniChar nul[3] = {0x41, 0, 0x42};
    memset(b, 0x55, sizeof b);
    CK(CFStringGetCString(CFStringCreateWithCharacters(kCFAllocatorDefault, nul, 3), b, 10, kCFStringEncodingUTF8));
    CK(mem_eq(b, "A\0B\0", 4) && (unsigned char)b[4] == 0x55);
    memset(b, 0x55, sizeof b);
    CK(CFStringGetCString(CFSTR("€"), b, 10, kCFStringEncodingUTF8));
    CK(mem_eq(b, "\xe2\x82\xac\0", 4));
    CK(CFStringGetMaximumSizeForEncoding(5, kCFStringEncodingUTF8) >= 15);
    CK(CFStringGetMaximumSizeForEncoding(5, kCFStringEncodingASCII) == 5);
    CK(CFStringGetMaximumSizeForEncoding(5, kCFStringEncodingUTF16) == 10);
    return finish();
}
