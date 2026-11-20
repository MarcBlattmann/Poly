/* poly-mac: iosurface
 * IORegistryEntryCreateCFProperty of a key the entry does not have is NULL; of an entry that is 0 or not a registry entry (0x1234) NULL; options 0 and a NULL allocator
 * are what Chrome passes; the value is a new reference the caller releases; two calls give equal values; IORegistryEntryCreateCFProperties gives a mutable dictionary with
 * the entry's properties (more than 10 for the platform expert). */
#include "poly_miosurface.h"
int test_main(void) {
    io_service_t pe = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOPlatformExpertDevice"));
    check("a key that does not exist: NULL", IORegistryEntryCreateCFProperty(pe, CFSTR("poly-no-such-key"), NULL, 0) == NULL, 0);
    check("entry 0: NULL", IORegistryEntryCreateCFProperty(0, CFSTR("model"), NULL, 0) == NULL, 0);
    check("entry 0x1234: NULL", IORegistryEntryCreateCFProperty(0x1234, CFSTR("model"), NULL, 0) == NULL, 0);
    CFTypeRef a = IORegistryEntryCreateCFProperty(pe, CFSTR("model"), NULL, 0), b = IORegistryEntryCreateCFProperty(pe, CFSTR("model"), kCFAllocatorDefault, 0);
    check("two calls give equal values, one with an allocator", a && b && CFEqual(a, b), 0);
    CFMutableDictionaryRef props = NULL;
    kern_return_t kr = IORegistryEntryCreateCFProperties(pe, &props, NULL, 0);
    check("IORegistryEntryCreateCFProperties: 0 and a dictionary", kr == KERN_SUCCESS && props && CFDictionaryGetCount(props) > 10, kr);
    check("it has the model", props && CFDictionaryContainsKey(props, CFSTR("model")), 0);
    return finish();
}
