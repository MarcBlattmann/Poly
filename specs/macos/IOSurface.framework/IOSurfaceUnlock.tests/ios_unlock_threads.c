/* poly-mac: iosurface
 * The lock is a count on the surface, not owned by a thread: a surface locked in one thread can be unlocked in another; memory written by one thread under the
 * lock is seen by the other after it. */
#include "poly_miosurface.h"
#include <pthread.h>
static IOSurfaceRef s;
static IOReturn unlock_result = -1;
static void *unlocker(void *a) {
    (void)a;
    unlock_result = IOSurfaceUnlock(s, 0, NULL);
    return NULL;
}
int test_main(void) {
    s = ios_make(64, 64, 4);
    IOSurfaceLock(s, 0, NULL);
    ((unsigned char *)IOSurfaceGetBaseAddress(s))[7] = 0x33;
    pthread_t t;
    pthread_create(&t, NULL, unlocker, NULL);
    pthread_join(t, NULL);
    check("an unlock in another thread: 0", unlock_result == 0, unlock_result);
    check("and nothing is left to unlock", IOSurfaceUnlock(s, 0, NULL) == (IOReturn)0xe00002c2, 0);
    check("the byte written is there", ((unsigned char *)IOSurfaceGetBaseAddress(s))[7] == 0x33, 0);
    return finish();
}
