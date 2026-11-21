/* poly-mac: iosurface
 * A GPU is found with IOServiceGetMatchingService of IOAccelerator: a service that conforms to IOAccelerator; its registry entry has a "model" property
 * that is a string, a "gpu-core-count" that is a number of at least 1 and a "vendor-id" that is data; the values are the machine's own and are not recorded. */
#include "poly_miosurface.h"
int test_main(void) {
    io_service_t gpu = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOAccelerator"));
    check("a service", gpu != 0, 0);
    check("it conforms to IOAccelerator", IOObjectConformsTo(gpu, "IOAccelerator"), 0);
    CFTypeRef model = IORegistryEntryCreateCFProperty(gpu, CFSTR("model"), NULL, 0);
    check("a model property that is a non-empty string", ios_str_len(model) > 0, ios_str_len(model));
    CFTypeRef cores = IORegistryEntryCreateCFProperty(gpu, CFSTR("gpu-core-count"), NULL, 0);
    check("a gpu-core-count that is a number of at least 1", cores && CFGetTypeID(cores) == CFNumberGetTypeID() && ios_num(cores) >= 1, 0);
    CFTypeRef vendor = IORegistryEntryCreateCFProperty(gpu, CFSTR("vendor-id"), NULL, 0);
    check("a vendor-id that is data of 4 bytes", vendor && CFGetTypeID(vendor) == CFDataGetTypeID() && CFDataGetLength(vendor) == 4, 0);
    value("the type of the model", strcmp(ios_type_name(model), "string") == 0);
    return finish();
}
