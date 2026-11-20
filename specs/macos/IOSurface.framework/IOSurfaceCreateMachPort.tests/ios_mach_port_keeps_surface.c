/* poly-mac: iosurface
 * A Mach port made with IOSurfaceCreateMachPort keeps the surface alive: after the last reference was released the surface still comes back from the port (with
 * its data), and IOSurfaceLookup by the ID still finds it; when the port's right is given back as well the ID gives NULL. */
#include "poly_miosurface.h"
int test_main(void) {
    IOSurfaceRef s = ios_make(64, 64, 4);
    uint32_t id = IOSurfaceGetID(s);
    IOSurfaceLock(s, 0, NULL);
    ((unsigned char *)IOSurfaceGetBaseAddress(s))[9] = 0x24;
    IOSurfaceUnlock(s, 0, NULL);
    mach_port_t mp = IOSurfaceCreateMachPort(s);
    CFRelease(s);
    IOSurfaceRef back = IOSurfaceLookupFromMachPort(mp);
    check("the surface comes back from the port", back != NULL && IOSurfaceGetID(back) == id, 0);
    IOSurfaceLock(back, kIOSurfaceLockReadOnly, NULL);
    check("with its data", ((unsigned char *)IOSurfaceGetBaseAddress(back))[9] == 0x24, 0);
    IOSurfaceUnlock(back, kIOSurfaceLockReadOnly, NULL);
    CFRelease(back);
    check("and IOSurfaceLookup finds it by its ID", IOSurfaceLookup(id) != NULL, 0);
    return finish();
}
