/* poly-mac: iosurface
 * What is written to the memory of a locked surface stays after the unlock and is seen through every reference to the surface (a second reference from
 * IOSurfaceLookup has the same base address and the same bytes); rows are BytesPerRow apart: a byte written at row 3, column 5 * 4 + 2 is at
 * base + 3 * BytesPerRow + 22. */
#include "poly_miosurface.h"
int test_main(void) {
    IOSurfaceRef s = ios_make(100, 50, 4);
    IOSurfaceLock(s, 0, NULL);
    unsigned char *b = IOSurfaceGetBaseAddress(s);
    size_t bpr = IOSurfaceGetBytesPerRow(s);
    for (int y = 0; y < 50; y++)
        for (int x = 0; x < 100; x++) b[y * bpr + x * 4] = (unsigned char)(x + y);
    IOSurfaceUnlock(s, 0, NULL);
    IOSurfaceLock(s, kIOSurfaceLockReadOnly, NULL);
    b = IOSurfaceGetBaseAddress(s);
    int ok = 1;
    for (int y = 0; y < 50; y++)
        for (int x = 0; x < 100; x++) ok &= b[y * bpr + x * 4] == (unsigned char)(x + y);
    check("every row, every column reads back after the unlock", ok, 0);
    check("row 3, column 5 is 8", b[3 * bpr + 5 * 4] == 8, b[3 * bpr + 5 * 4]);
    IOSurfaceUnlock(s, kIOSurfaceLockReadOnly, NULL);
    IOSurfaceRef again = IOSurfaceLookup(IOSurfaceGetID(s));
    check("a second reference by ID", again != NULL, 0);
    if (again) {
        check("has the same base address", IOSurfaceGetBaseAddress(again) == IOSurfaceGetBaseAddress(s), 0);
        check("and the same bytes", ((unsigned char *)IOSurfaceGetBaseAddress(again))[3 * bpr + 5 * 4] == 8, 0);
    }
    return finish();
}
