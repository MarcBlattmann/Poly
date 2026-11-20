/* poly-mac: iosurface
 * IOSurfaceCopyValue of a key nobody set is NULL; of the keys the surface was made with (kIOSurfaceWidth, kIOSurfaceHeight, kIOSurfaceBytesPerElement,
 * kIOSurfaceBytesPerRow, kIOSurfaceAllocSize, kIOSurfacePixelFormat) it is NULL too: they are read with the IOSurfaceGet functions; of kIOSurfaceName, set at
 * creation, it is the name; an attachment removed with IOSurfaceRemoveValue is NULL, the name is not removed. */
#include "poly_miosurface.h"
int test_main(void) {
    CFMutableDictionaryRef d = ios_props(64, 64, 4, 'BGRA');
    CFDictionarySetValue(d, kIOSurfaceName, CFSTR("poly-name"));
    IOSurfaceRef s = ios_make_props(d);
    check("a key nobody set: NULL", IOSurfaceCopyValue(s, CFSTR("poly.nothing")) == NULL, 0);
    check("kIOSurfaceWidth: NULL", IOSurfaceCopyValue(s, kIOSurfaceWidth) == NULL, 0);
    check("kIOSurfaceHeight: NULL", IOSurfaceCopyValue(s, kIOSurfaceHeight) == NULL, 0);
    check("kIOSurfaceBytesPerElement: NULL", IOSurfaceCopyValue(s, kIOSurfaceBytesPerElement) == NULL, 0);
    check("kIOSurfaceBytesPerRow: NULL", IOSurfaceCopyValue(s, kIOSurfaceBytesPerRow) == NULL, 0);
    check("kIOSurfaceAllocSize: NULL", IOSurfaceCopyValue(s, kIOSurfaceAllocSize) == NULL, 0);
    check("kIOSurfacePixelFormat: NULL", IOSurfaceCopyValue(s, kIOSurfacePixelFormat) == NULL, 0);
    check("kIOSurfaceName: the name", ios_str_eq(IOSurfaceCopyValue(s, kIOSurfaceName), "poly-name"), 0);
    IOSurfaceSetValue(s, CFSTR("poly.extra"), CFSTR("x"));
    IOSurfaceRemoveValue(s, CFSTR("poly.extra"));
    check("an attachment removed with IOSurfaceRemoveValue is NULL", IOSurfaceCopyValue(s, CFSTR("poly.extra")) == NULL, 0);
    IOSurfaceRemoveValue(s, kIOSurfaceName);
    check("the name given at creation is not removed by IOSurfaceRemoveValue", ios_str_eq(IOSurfaceCopyValue(s, kIOSurfaceName), "poly-name"), 0);
    check("the empty key: NULL", IOSurfaceCopyValue(s, CFSTR("")) == NULL, 0);
    return finish();
}
