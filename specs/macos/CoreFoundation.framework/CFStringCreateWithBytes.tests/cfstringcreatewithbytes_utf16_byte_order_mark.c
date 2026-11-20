/* poly-mac: corefoundation */
/* A byte order mark selects the byte order and is consumed only when the bytes
 * are an external representation and the encoding is plain UTF-16; for
 * the explicit LE and BE encodings, and for a non-external UTF-16, it stays
 * as the character U+FEFF. */
#include "poly_mcf.h"

static CFStringRef mk(const unsigned char *b, CFIndex n, CFStringEncoding e, Boolean ext) {
    return CFStringCreateWithBytes(kCFAllocatorDefault, b, n, e, ext);
}

int test_main(void) {
    static const unsigned char bomle[] = {0xff, 0xfe, 0x61, 0}, bombe[] = {0xfe, 0xff, 0, 0x61};
    CK(UNITS(mk(bomle, 4, kCFStringEncodingUTF16, 1), 0x61));
    CK(UNITS(mk(bombe, 4, kCFStringEncodingUTF16, 1), 0x61));
    CK(UNITS(mk(bomle, 4, kCFStringEncodingUTF16, 0), 0xfeff, 0x61));
    CK(UNITS(mk(bomle, 4, kCFStringEncodingUTF16LE, 0), 0xfeff, 0x61));
    CK(UNITS(mk(bomle, 4, kCFStringEncodingUTF16LE, 1), 0xfeff, 0x61));
    return finish();
}
