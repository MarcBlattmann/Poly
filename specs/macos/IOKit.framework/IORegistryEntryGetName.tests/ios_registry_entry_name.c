/* poly-mac: iosurface
 * IORegistryEntryGetName(entry, name) stores the entry's name in an io_name_t (128 bytes) and returns 0: non-empty, shorter than 128 characters; for any service the name
 * is not the class name necessarily (the platform expert's name is the machine's model identifier: never printed here); IOObjectGetClass gives the class: for the platform
 * expert 'IOPlatformExpertDevice'; a GPU's class starts with the letters of its family. */
#include "poly_miosurface.h"
int test_main(void) {
    io_service_t pe = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOPlatformExpertDevice"));
    io_name_t name;
    memset(name, 0x7f, sizeof name);
    kern_return_t kr = IORegistryEntryGetName(pe, name);
    check("returns 0", kr == KERN_SUCCESS, kr);
    size_t n = strnlen(name, sizeof name);
    check("a name of 1 to 127 characters", n >= 1 && n < sizeof name, n);
    io_name_t cls;
    check("the class of the platform expert", IOObjectGetClass(pe, cls) == KERN_SUCCESS && strcmp(cls, "IOPlatformExpertDevice") == 0, 0);
    io_service_t gpu = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOAccelerator"));
    io_name_t gname;
    check("a GPU also has a name", IORegistryEntryGetName(gpu, gname) == KERN_SUCCESS && strlen(gname) >= 1, 0);
    return finish();
}
