/* poly-mac: iosurface
 * A matching dictionary passed to IOServiceGetMatchingServices or IOServiceGetMatchingService is consumed: the caller does not release it afterwards (releasing it
 * again would be a mistake); the same dictionary can be used by retaining it first: two searches with one retained dictionary find the same thing. */
#include "poly_miosurface.h"
int test_main(void) {
    CFMutableDictionaryRef d = IOServiceMatching("IOPlatformExpertDevice");
    CFRetain(d);
    io_service_t a = IOServiceGetMatchingService(kIOMainPortDefault, d);
    io_service_t b = IOServiceGetMatchingService(kIOMainPortDefault, d);
    check("two searches with one retained dictionary both find the platform expert", a != 0 && b != 0, 0);
    check("and it is the same registry entry", IOObjectIsEqualTo(a, b), 0);
    IOObjectRelease(a);
    IOObjectRelease(b);
    return finish();
}
