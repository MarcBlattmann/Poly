/* poly-mac: corefoundation */
/* A UTF-16 byte count that is odd drops the last byte (a single byte gives
 * NULL); UTF-32 gives surrogate pairs above U+FFFF, in either byte order. */
#include "poly_mcf.h"

int test_main(void) {
    static const unsigned char le[] = {0x61, 0, 0x62, 0};
    CK(UNITS(CFStringCreateWithBytes(kCFAllocatorDefault, le, 3, kCFStringEncodingUTF16LE, 0), 0x61));
    CK(CFStringCreateWithBytes(kCFAllocatorDefault, le, 1, kCFStringEncodingUTF16LE, 0) == NULL);
    static const unsigned char u32[] = {0x41, 0, 0, 0, 0x00, 0xf6, 0x01, 0};
    CK(UNITS(CFStringCreateWithBytes(kCFAllocatorDefault, u32, 8, kCFStringEncodingUTF32LE, 0), 0x41, 0xd83d, 0xde00));
    static const unsigned char u32be[] = {0, 0, 0, 0x41, 0, 1, 0xf6, 0};
    CK(UNITS(CFStringCreateWithBytes(kCFAllocatorDefault, u32be, 8, kCFStringEncodingUTF32BE, 0), 0x41, 0xd83d, 0xde00));
    return finish();
}
