/* poly-mac: iosurface
 * A lock is on the surface, not on the reference: a second reference from IOSurfaceLookup sees the seed the first lock made, a lock on it raises the same seed. */
#include "poly_miosurface.h"
int test_main(void) {
    IOSurfaceRef a = ios_make(64, 64, 4);
    IOSurfaceRef b = IOSurfaceLookup(IOSurfaceGetID(a));
    check("a second reference", b != NULL, 0);
    IOSurfaceLock(a, 0, NULL);
    IOSurfaceUnlock(a, 0, NULL);
    check("the second reference sees seed 1", IOSurfaceGetSeed(b) == 1, IOSurfaceGetSeed(b));
    uint32_t seed = 0;
    IOSurfaceLock(b, 0, &seed);
    IOSurfaceUnlock(b, 0, NULL);
    check("a lock on it makes 2", seed == 2, seed);
    check("and the first reference sees 2", IOSurfaceGetSeed(a) == 2, IOSurfaceGetSeed(a));
    return finish();
}
