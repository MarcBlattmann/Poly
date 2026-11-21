/* poly-mac: iosurface
 * IOSurfaceGetBytesPerRow reports the BytesPerRow given at creation unchanged (not aligned) and the property read back with IOSurfaceCopyValue is
 * not stored (the built-in keys, width, height, bytes per row, allocation size, are not among the values IOSurfaceCopyValue returns). */
#include "poly_miosurface.h"
int test_main(void) {
    CFMutableDictionaryRef d = ios_props(100, 10, 4, 'BGRA');
    ios_set_int(d, kIOSurfaceBytesPerRow, 1000);
    IOSurfaceRef s = ios_make_props(d);
    check("1000 is reported as 1000", s && IOSurfaceGetBytesPerRow(s) == 1000, 0);
    IOSurfaceRef plain = ios_make(100, 10, 4);
    CFTypeRef v = IOSurfaceCopyValue(plain, kIOSurfaceBytesPerRow);
    check("CopyValue of kIOSurfaceBytesPerRow is NULL", v == NULL, 0);
    check("CopyValue of kIOSurfaceWidth is NULL", IOSurfaceCopyValue(plain, kIOSurfaceWidth) == NULL, 0);
    check("CopyValue of kIOSurfaceAllocSize is NULL", IOSurfaceCopyValue(plain, kIOSurfaceAllocSize) == NULL, 0);
    check("the BytesPerRow of the surface made without one is aligned", IOSurfaceGetBytesPerRow(plain) == 512, IOSurfaceGetBytesPerRow(plain));
    return finish();
}
