/* poly-mac: iosurface
 * IOSurfaceGetBaseAddress of a surface that is not locked is not NULL and is the same pointer before, during and after a lock and on every call; it is aligned to
 * 4096 bytes (to 16384 on the kernel here), the whole allocation can be read and a fresh surface is all zero. */
#include "poly_miosurface.h"
int test_main(void) {
    IOSurfaceRef s = ios_make(100, 50, 4);
    void *p = IOSurfaceGetBaseAddress(s);
    check("not NULL while unlocked", p != NULL, 0);
    check("the same on every call", IOSurfaceGetBaseAddress(s) == p, 0);
    check("aligned to 4096", ((uintptr_t)p & 4095) == 0, 0);
    value("aligned to 16384", ((uintptr_t)p & 16383) == 0);
    check("the same while locked", IOSurfaceLock(s, 0, NULL) == KERN_SUCCESS && IOSurfaceGetBaseAddress(s) == p, 0);
    unsigned char *b = p;
    size_t n = IOSurfaceGetAllocSize(s);
    int zero = 1;
    for (size_t i = 0; i < n; i++)
        if (b[i]) {
            zero = 0;
            break;
        }
    check("a fresh surface is all zero over its whole allocation", zero, 0);
    IOSurfaceUnlock(s, 0, NULL);
    check("the same after the unlock", IOSurfaceGetBaseAddress(s) == p, 0);
    IOSurfaceRef t = ios_make(100, 50, 4);
    check("another surface has another address", IOSurfaceGetBaseAddress(t) != p, 0);
    return finish();
}
