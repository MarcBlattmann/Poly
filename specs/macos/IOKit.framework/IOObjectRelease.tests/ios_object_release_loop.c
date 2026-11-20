/* poly-mac: iosurface
 * Objects released are given back: a thousand searches for the platform expert, each released, work every time (the names are reused, no search fails), and a thousand
 * iterators over a class, each drained and released, the same. */
#include "poly_miosurface.h"
int test_main(void) {
    int ok = 1;
    for (int i = 0; i < 1000; i++) {
        io_service_t s = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOPlatformExpertDevice"));
        ok &= s != 0 && IOObjectRelease(s) == KERN_SUCCESS;
    }
    check("a thousand searches and releases", ok, 0);
    ok = 1;
    for (int i = 0; i < 1000; i++) {
        io_iterator_t it = 0;
        ok &= IOServiceGetMatchingServices(kIOMainPortDefault, IOServiceMatching("IOSurfaceRoot"), &it) == KERN_SUCCESS && ios_drain(it) == 1;
    }
    check("a thousand iterators drained and released", ok, 0);
    return finish();
}
