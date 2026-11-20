/* poly-mac: iosurface
 * IOServiceGetMatchingService(kIOMainPortDefault, matching) returns the first service of the class, not 0 (an io_service_t to release with IOObjectRelease);
 * it is the entry the iterator gives first; it conforms to the class; its class name (IOObjectGetClass) is that of the class asked for. */
#include "poly_miosurface.h"
int test_main(void) {
    io_service_t s = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOPlatformExpertDevice"));
    check("a service", s != 0, 0);
    check("it conforms to IOPlatformExpertDevice and to IOService", IOObjectConformsTo(s, "IOPlatformExpertDevice") && IOObjectConformsTo(s, "IOService"), 0);
    io_name_t cls;
    check("IOObjectGetClass succeeds", IOObjectGetClass(s, cls) == KERN_SUCCESS, 0);
    check("the class is IOPlatformExpertDevice", strcmp(cls, "IOPlatformExpertDevice") == 0, 0);
    io_iterator_t it = 0;
    IOServiceGetMatchingServices(kIOMainPortDefault, IOServiceMatching("IOPlatformExpertDevice"), &it);
    io_object_t first = IOIteratorNext(it);
    check("the same registry entry as the iterator's first", first && IOObjectIsEqualTo(s, first), 0);
    uint64_t id1 = 0, id2 = 0;
    IORegistryEntryGetRegistryEntryID(s, &id1);
    IORegistryEntryGetRegistryEntryID(first, &id2);
    check("with the same registry entry ID, not 0", id1 != 0 && id1 == id2, 0);
    return finish();
}
