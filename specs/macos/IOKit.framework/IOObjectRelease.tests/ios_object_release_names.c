/* poly-mac: iosurface
 * The object names IOKit hands out are Mach port names in this task: an io_service_t of a search is a send right (type 0x10000), searching twice gives the same name for the
 * same entry, and after the last release the name is invalid. */
#include "poly_miosurface.h"
static unsigned type_of(mach_port_t n) {
    mach_port_type_t t = 0;
    kern_return_t kr = mach_port_type(mach_task_self(), n, &t);
    return kr ? 0xdead0000u | (unsigned)kr : (unsigned)t;
}
int test_main(void) {
    io_service_t a = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOPlatformExpertDevice"));
    check("the object is a send right", type_of(a) == MACH_PORT_TYPE_SEND, type_of(a));
    io_service_t b = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOPlatformExpertDevice"));
    check("the same name for the same entry", a == b, 0);
    IOObjectRelease(a);
    IOObjectRelease(b);
    check("after the last release the name is invalid", type_of(a) == (0xdead0000u | KERN_INVALID_NAME), type_of(a));
    return finish();
}
