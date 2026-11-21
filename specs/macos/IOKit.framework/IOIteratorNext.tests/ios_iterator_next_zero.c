/* poly-mac: iosurface
 * IOIteratorNext(0) returns 0; IOObjectRelease(0) returns 0 (nothing to do); an object released twice: the second release returns 15 (KERN_INVALID_NAME). */
#include "poly_miosurface.h"
int test_main(void) {
    check("IOIteratorNext(0): 0", IOIteratorNext(0) == 0, 0);
    check("IOObjectRelease(0): 0", IOObjectRelease(0) == KERN_SUCCESS, 0);
    io_service_t s = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOPlatformExpertDevice"));
    check("IOObjectRelease of a service: 0", IOObjectRelease(s) == KERN_SUCCESS, 0);
    check("released again: 15", IOObjectRelease(s) == KERN_INVALID_NAME, 0);
    return finish();
}
