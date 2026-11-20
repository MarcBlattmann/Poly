/* poly-mac: corefoundation */
/* UTF-16 bytes: little and big endian read as named; the unmarked form takes
 * the machine's order (little endian here) unless the bytes are an external
 * representation, which is big endian without a byte order mark. */
#include "poly_mcf.h"

static CFStringRef mk(const unsigned char *b, CFIndex n, CFStringEncoding e, Boolean ext) {
    return CFStringCreateWithBytes(kCFAllocatorDefault, b, n, e, ext);
}

int test_main(void) {
    static const unsigned char le[] = {0x61, 0, 0x62, 0}, be[] = {0, 0x61, 0, 0x62};
    CK(UNITS(mk(le, 4, kCFStringEncodingUTF16LE, 0), 0x61, 0x62));
    CK(UNITS(mk(be, 4, kCFStringEncodingUTF16BE, 0), 0x61, 0x62));
    CK(UNITS(mk(le, 4, kCFStringEncodingUTF16, 0), 0x61, 0x62));
    CK(UNITS(mk(be, 4, kCFStringEncodingUTF16, 1), 0x61, 0x62));
    CK(UNITS(mk(le, 4, kCFStringEncodingUTF16LE, 1), 0x61, 0x62));
    CK(UNITS(mk(be, 4, kCFStringEncodingUTF16BE, 1), 0x61, 0x62));
    return finish();
}
