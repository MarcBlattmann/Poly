/* poly-mac: iosurface
 * The count is kept per surface and process, not per reference: increments made through the original and through a reference got by IOSurfaceLookupFromMachPort and
 * by IOSurfaceLookup add up; releasing a reference does not change the count. */
#include "poly_miosurface.h"
int test_main(void) {
    IOSurfaceRef a = ios_make(32, 32, 4);
    mach_port_t mp = IOSurfaceCreateMachPort(a);
    IOSurfaceRef b = IOSurfaceLookupFromMachPort(mp), c = IOSurfaceLookup(IOSurfaceGetID(a));
    IOSurfaceIncrementUseCount(a);
    IOSurfaceIncrementUseCount(b);
    IOSurfaceIncrementUseCount(c);
    check("three increments through three references: 3 on each", IOSurfaceGetUseCount(a) == 3 && IOSurfaceGetUseCount(b) == 3 && IOSurfaceGetUseCount(c) == 3, IOSurfaceGetUseCount(a));
    CFRelease(b);
    check("releasing a reference does not change the count", IOSurfaceGetUseCount(a) == 3, IOSurfaceGetUseCount(a));
    return finish();
}
