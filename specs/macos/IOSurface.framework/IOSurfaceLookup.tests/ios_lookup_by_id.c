/* poly-mac: iosurface
 * IOSurfaceLookup(id) in the process that made the surface gives a new reference to it, found by its ID, though the surface is not global: same ID, same sizes, same
 * base address, same memory (a byte written through one is read through the other); the reference is another object; an ID that is nothing (0, 0xdeadbeef, a
 * small number nobody has) gives NULL. */
#include "poly_miosurface.h"
int test_main(void) {
    IOSurfaceRef a = ios_make(64, 64, 4);
    IOSurfaceRef b = IOSurfaceLookup(IOSurfaceGetID(a));
    check("found", b != NULL, 0);
    check("another object for the same surface", b != a, 0);
    check("same ID, width, height, BytesPerRow", IOSurfaceGetID(b) == IOSurfaceGetID(a) && IOSurfaceGetWidth(b) == 64 && IOSurfaceGetHeight(b) == 64 && IOSurfaceGetBytesPerRow(b) == IOSurfaceGetBytesPerRow(a), 0);
    check("same base address", IOSurfaceGetBaseAddress(b) == IOSurfaceGetBaseAddress(a), 0);
    IOSurfaceLock(a, 0, NULL);
    ((unsigned char *)IOSurfaceGetBaseAddress(a))[5] = 0x6c;
    IOSurfaceUnlock(a, 0, NULL);
    check("a byte written through a is read through b", ((unsigned char *)IOSurfaceGetBaseAddress(b))[5] == 0x6c, 0);
    check("ID 0: NULL", IOSurfaceLookup(0) == NULL, 0);
    check("ID 0xdeadbeef: NULL", IOSurfaceLookup(0xdeadbeef) == NULL, 0);
    check("ID 1: NULL", IOSurfaceLookup(1) == NULL, 0);
    return finish();
}
