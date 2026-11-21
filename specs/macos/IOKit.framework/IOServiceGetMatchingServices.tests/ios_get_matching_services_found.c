/* poly-mac: iosurface
 * IOServiceGetMatchingServices(kIOMainPortDefault, matching, &iterator) returns 0 (KERN_SUCCESS) and an iterator over the services of the class (and its subclasses);
 * the iterator gives one for IOPlatformExpertDevice and then 0; the matching dictionary is consumed; the main port argument kIOMainPortDefault is 0. */
#include "poly_miosurface.h"
int test_main(void) {
    io_iterator_t it = 0;
    kern_return_t kr = IOServiceGetMatchingServices(kIOMainPortDefault, IOServiceMatching("IOPlatformExpertDevice"), &it);
    check("returns 0", kr == KERN_SUCCESS, kr);
    check("an iterator", it != 0, 0);
    int n = 0;
    io_object_t o;
    while ((o = IOIteratorNext(it))) {
        n++;
        check("the object conforms to IOPlatformExpertDevice", IOObjectConformsTo(o, "IOPlatformExpertDevice"), 0);
        IOObjectRelease(o);
    }
    check("exactly one service", n == 1, n);
    IOObjectRelease(it);
    check("kIOMainPortDefault is 0", kIOMainPortDefault == 0, kIOMainPortDefault);
    return finish();
}
