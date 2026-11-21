/* poly-mac: iosurface
 * IORegistryEntryGetRegistryEntryID gives a 64-bit ID that is not 0, the same on every call and for every object that refers to the entry; two different entries (the
 * platform expert and a GPU) have different IDs; IOObjectIsEqualTo is true for two objects of the same entry and false for different ones. */
#include "poly_miosurface.h"
int test_main(void) {
    io_service_t a = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOPlatformExpertDevice"));
    io_service_t b = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOPlatformExpertDevice"));
    io_service_t gpu = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOAccelerator"));
    uint64_t ia = 0, ib = 0, ig = 0, ia2 = 0;
    check("the calls return 0", IORegistryEntryGetRegistryEntryID(a, &ia) == 0 && IORegistryEntryGetRegistryEntryID(b, &ib) == 0 && IORegistryEntryGetRegistryEntryID(gpu, &ig) == 0 && IORegistryEntryGetRegistryEntryID(a, &ia2) == 0, 0);
    check("not 0 and the same for the same entry", ia != 0 && ia == ib && ia == ia2, 0);
    check("different for different entries", ia != ig, 0);
    check("IOObjectIsEqualTo agrees", IOObjectIsEqualTo(a, b) && !IOObjectIsEqualTo(a, gpu), 0);
    return finish();
}
