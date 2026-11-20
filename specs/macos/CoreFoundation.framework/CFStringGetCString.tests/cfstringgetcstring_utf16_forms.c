/* poly-mac: corefoundation */
/* UTF-16 as a C-string encoding: LE and BE give the units in that order
 * with no byte order mark, plain UTF-16 puts a mark first and uses little
 * endian; one NUL byte ends the data. */
#include "poly_mcf.h"

int test_main(void) {
    CFStringRef a = S("hello");
    char b[32];
    memset(b, 0x55, sizeof b);
    CK(CFStringGetCString(a, b, 20, kCFStringEncodingUTF16LE));
    CK(mem_eq(b, "h\0e\0l\0l\0o\0\0", 11) && (unsigned char)b[11] == 0x55);
    memset(b, 0x55, sizeof b);
    CK(CFStringGetCString(a, b, 20, kCFStringEncodingUTF16BE));
    CK(mem_eq(b, "\0h\0e\0l\0l\0o\0", 11) && (unsigned char)b[11] == 0x55);
    memset(b, 0x55, sizeof b);
    CK(CFStringGetCString(a, b, 20, kCFStringEncodingUTF16));
    CK(mem_eq(b, "\xff\xfeh\0e\0l\0l\0o\0\0", 13) && (unsigned char)b[13] == 0x55);
    return finish();
}
