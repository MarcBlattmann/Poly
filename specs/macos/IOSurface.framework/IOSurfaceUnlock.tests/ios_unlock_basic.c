/* poly-mac: iosurface
 * IOSurfaceUnlock returns 0 for each lock held and the options and seed arguments may be anything (the options of the lock are not checked to match, NULL for the
 * seed); an unlock that has no lock to undo returns 0xe00002c2 (kIOReturnBadArgument in IOKit's numbering) and changes nothing; after it the surface can be locked and
 * unlocked as before. */
#include "poly_miosurface.h"
int test_main(void) {
    IOSurfaceRef s = ios_make(64, 64, 4);
    check("an unlock with no lock: 0xe00002c2", IOSurfaceUnlock(s, 0, NULL) == (IOReturn)0xe00002c2, IOSurfaceUnlock(s, 0, NULL));
    check("the seed did not change", IOSurfaceGetSeed(s) == 0, IOSurfaceGetSeed(s));
    IOSurfaceLock(s, 0, NULL);
    check("an unlock with the lock: 0", IOSurfaceUnlock(s, 0, NULL) == 0, 0);
    check("a second unlock: 0xe00002c2", IOSurfaceUnlock(s, 0, NULL) == (IOReturn)0xe00002c2, 0);
    IOSurfaceLock(s, 0, NULL);
    IOSurfaceLock(s, 0, NULL);
    check("two locks, first unlock: 0", IOSurfaceUnlock(s, 0, NULL) == 0, 0);
    check("second unlock: 0", IOSurfaceUnlock(s, 0, NULL) == 0, 0);
    check("third unlock: 0xe00002c2", IOSurfaceUnlock(s, 0, NULL) == (IOReturn)0xe00002c2, 0);
    check("the surface can be locked again", IOSurfaceLock(s, 0, NULL) == 0 && IOSurfaceUnlock(s, 0, NULL) == 0, 0);
    return finish();
}
