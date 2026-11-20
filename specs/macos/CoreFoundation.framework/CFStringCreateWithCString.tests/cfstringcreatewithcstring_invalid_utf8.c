/* poly-mac: corefoundation */
/* Bytes that are not well-formed UTF-8 give NULL: stray continuation or lead
 * bytes, a truncated sequence, an overlong form, an encoded surrogate and a
 * code point beyond U+10FFFF. */
#include "poly_mcf.h"

#define U8(s) CFStringCreateWithCString(kCFAllocatorDefault, s, kCFStringEncodingUTF8)

int test_main(void) {
    CK(U8("\xff\xfe") == NULL);
    CK(U8("\x80") == NULL);
    CK(U8("ab\xc3") == NULL);
    CK(U8("\xc0\x80") == NULL);
    CK(U8("\xe0\x80\x80") == NULL);
    CK(U8("\xed\xa0\x80") == NULL);
    CK(U8("\xf4\x90\x80\x80") == NULL);
    CK(U8("\xf0\x9f\x98") == NULL);
    CK(U8("fine") != NULL);
    return finish();
}
