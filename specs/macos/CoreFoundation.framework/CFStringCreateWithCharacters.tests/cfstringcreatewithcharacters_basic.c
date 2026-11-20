/* poly-mac: corefoundation */
/* The characters are taken as they are: no normalisation, no byte order mark
 * processing, surrogates and NUL included. */
#include "poly_mcf.h"

#define CH(...) CFStringCreateWithCharacters(kCFAllocatorDefault, (const UniChar[]){__VA_ARGS__}, \
                                             (CFIndex)(sizeof((const UniChar[]){__VA_ARGS__}) / sizeof(UniChar)))

int test_main(void) {
    CK(UNITS(CH(0x41, 0x42), 0x41, 0x42));
    CK(UNITS(CH(0x41, 0xd83d, 0xde00), 0x41, 0xd83d, 0xde00));
    CK(UNITS(CH(0xd83d), 0xd83d));
    CK(UNITS(CH(0xde00, 0xd83d), 0xde00, 0xd83d));
    CK(UNITS(CH(0, 0x41), 0, 0x41));
    CK(UNITS(CH(0x65, 0x301), 0x65, 0x301));
    CK(UNITS(CH(0xfeff, 0x61), 0xfeff, 0x61));
    CK(UNITS(CH(0xffff), 0xffff));
    return finish();
}
