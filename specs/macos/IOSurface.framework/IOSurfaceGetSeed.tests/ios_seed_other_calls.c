/* poly-mac: iosurface
 * Calls that are not a read-write lock leave the seed alone: writing to the memory, IOSurfaceSetValue, IOSurfaceCopyValue, IOSurfaceIncrementUseCount,
 * IOSurfaceDecrementUseCount, IOSurfaceCreateMachPort, IOSurfaceLookupFromMachPort, IOSurfaceLookup. */
#include "poly_miosurface.h"
int test_main(void) {
    IOSurfaceRef s = ios_make(32, 32, 4);
    IOSurfaceLock(s, 0, NULL);
    ((unsigned char *)IOSurfaceGetBaseAddress(s))[0] = 1;
    IOSurfaceUnlock(s, 0, NULL);
    uint32_t seed = IOSurfaceGetSeed(s);
    check("a lock and a write made seed 1", seed == 1, seed);
    ((unsigned char *)IOSurfaceGetBaseAddress(s))[1] = 2;
    check("a write without a lock does not raise it", IOSurfaceGetSeed(s) == seed, IOSurfaceGetSeed(s));
    CFStringRef k = ios_cfstr("poly.k");
    IOSurfaceSetValue(s, k, CFSTR("v"));
    check("IOSurfaceSetValue does not", IOSurfaceGetSeed(s) == seed, IOSurfaceGetSeed(s));
    CFTypeRef v = IOSurfaceCopyValue(s, k);
    check("IOSurfaceCopyValue does not", v && IOSurfaceGetSeed(s) == seed, IOSurfaceGetSeed(s));
    IOSurfaceIncrementUseCount(s);
    IOSurfaceDecrementUseCount(s);
    check("the use count calls do not", IOSurfaceGetSeed(s) == seed, IOSurfaceGetSeed(s));
    mach_port_t mp = IOSurfaceCreateMachPort(s);
    IOSurfaceRef other = IOSurfaceLookupFromMachPort(mp);
    check("creating and looking up a Mach port do not", other && IOSurfaceGetSeed(s) == seed && IOSurfaceGetSeed(other) == seed, 0);
    IOSurfaceRef byid = IOSurfaceLookup(IOSurfaceGetID(s));
    check("IOSurfaceLookup does not", byid && IOSurfaceGetSeed(byid) == seed, 0);
    return finish();
}
