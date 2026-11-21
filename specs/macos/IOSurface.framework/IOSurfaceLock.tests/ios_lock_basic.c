/* poly-mac: iosurface
 * IOSurfaceLock(surface, options, &seed) returns 0 (kIOReturnSuccess) with options 0 (read and write), kIOSurfaceLockReadOnly (1) and kIOSurfaceLockAvoidSync (2) and
 * with an option bit that means nothing (0x80); the seed pointer may be NULL; locks nest (two locks, two unlocks); a read-only lock and a read-write lock can be held at
 * the same time by one thread. */
#include "poly_miosurface.h"
int test_main(void) {
    IOSurfaceRef s = ios_make(64, 64, 4);
    check("options 0", IOSurfaceLock(s, 0, NULL) == 0, 0);
    check("and unlock", IOSurfaceUnlock(s, 0, NULL) == 0, 0);
    check("kIOSurfaceLockReadOnly", IOSurfaceLock(s, kIOSurfaceLockReadOnly, NULL) == 0, 0);
    check("and unlock", IOSurfaceUnlock(s, kIOSurfaceLockReadOnly, NULL) == 0, 0);
    check("kIOSurfaceLockAvoidSync", IOSurfaceLock(s, kIOSurfaceLockAvoidSync, NULL) == 0, 0);
    check("and unlock", IOSurfaceUnlock(s, kIOSurfaceLockAvoidSync, NULL) == 0, 0);
    check("an option bit that means nothing (0x80)", IOSurfaceLock(s, 0x80, NULL) == 0, 0);
    IOSurfaceUnlock(s, 0x80, NULL);
    check("nested: two locks", IOSurfaceLock(s, 0, NULL) == 0 && IOSurfaceLock(s, 0, NULL) == 0, 0);
    check("two unlocks", IOSurfaceUnlock(s, 0, NULL) == 0 && IOSurfaceUnlock(s, 0, NULL) == 0, 0);
    check("a read-only lock and a read-write lock together", IOSurfaceLock(s, kIOSurfaceLockReadOnly, NULL) == 0 && IOSurfaceLock(s, 0, NULL) == 0, 0);
    IOSurfaceUnlock(s, 0, NULL);
    IOSurfaceUnlock(s, kIOSurfaceLockReadOnly, NULL);
    return finish();
}
