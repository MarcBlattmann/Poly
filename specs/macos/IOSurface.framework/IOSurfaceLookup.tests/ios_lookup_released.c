/* poly-mac: iosurface
 * IOSurfaceLookup finds a surface as long as something holds it: a reference, or a Mach port made with IOSurfaceCreateMachPort. After the only reference was
 * released and no port exists the surface is gone and the ID gives NULL. */
#include "poly_miosurface.h"
int test_main(void) {
    IOSurfaceRef s = ios_make(64, 64, 4);
    uint32_t id = IOSurfaceGetID(s);
    CFRelease(s);
    check("after the only reference was released (no port): NULL", IOSurfaceLookup(id) == NULL, 0);
    IOSurfaceRef t = ios_make(64, 64, 4);
    uint32_t tid = IOSurfaceGetID(t);
    mach_port_t mp = IOSurfaceCreateMachPort(t);
    CFRelease(t);
    IOSurfaceRef found = IOSurfaceLookup(tid);
    check("with a Mach port held: found after the reference was released", found != NULL && IOSurfaceGetID(found) == tid, 0);
    value("the surface comes back by the port too", IOSurfaceLookupFromMachPort(mp) != NULL);
    return finish();
}
