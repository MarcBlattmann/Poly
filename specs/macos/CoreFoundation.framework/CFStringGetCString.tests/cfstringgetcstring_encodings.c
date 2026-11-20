/* poly-mac: corefoundation */
/* The same text in several encodings: Latin 1 and Mac Roman have one byte for
 * é, UTF-8 two, ASCII none (the call fails). */
#include "poly_mcf.h"

int test_main(void) {
    CFStringRef e = S("h\xc3\xa9llo");
    char b[16];
    memset(b, 0, sizeof b);
    CK(CFStringGetCString(e, b, 10, kCFStringEncodingISOLatin1));
    CK(mem_eq(b, "h\xe9llo", 6));
    memset(b, 0, sizeof b);
    CK(CFStringGetCString(e, b, 10, kCFStringEncodingMacRoman));
    CK(mem_eq(b, "h\x8ello", 6));
    memset(b, 0, sizeof b);
    CK(CFStringGetCString(e, b, 10, kCFStringEncodingWindowsLatin1));
    CK(mem_eq(b, "h\xe9llo", 6));
    CK(!CFStringGetCString(e, b, 10, kCFStringEncodingASCII));
    UniChar euro[1] = {0x20ac};
    CFStringRef eu = CFStringCreateWithCharacters(kCFAllocatorDefault, euro, 1);
    memset(b, 0, sizeof b);
    CK(CFStringGetCString(eu, b, 5, kCFStringEncodingWindowsLatin1));
    CK((unsigned char)b[0] == 0x80 && b[1] == 0);
    CK(!CFStringGetCString(eu, b, 5, kCFStringEncodingISOLatin1));
    memset(b, 0, sizeof b);
    CK(CFStringGetCString(eu, b, 5, kCFStringEncodingUTF8));
    CK(mem_eq(b, "\xe2\x82\xac", 4));
    return finish();
}
