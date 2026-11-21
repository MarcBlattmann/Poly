/* poly-mac: iosurface
 * IOSurfaceCreate returns NULL for properties that do not make a surface: no width, no height, width or height 0 or negative, bytes per element 0, an empty
 * dictionary, a BytesPerRow smaller than width * bytes per element (399 for 100 * 4), an AllocSize smaller than the surface needs, a size too big
 * to make (100000 by 100000 elements of 4 bytes), a plane info array with no plane in it. A surface with a width of 65536 is made. */
#include "poly_miosurface.h"
static int refused(CFMutableDictionaryRef d) {
    IOSurfaceRef s = IOSurfaceCreate(d);
    CFRelease(d);
    return s == NULL;
}
int test_main(void) {
    check("no width", refused(ios_props(-2000000, 10, 4, 'BGRA')), 0);
    check("no height", refused(ios_props(100, -2000000, 4, 'BGRA')), 0);
    check("width 0", refused(ios_props(0, 10, 4, 'BGRA')), 0);
    check("height 0", refused(ios_props(100, 0, 4, 'BGRA')), 0);
    check("width -5", refused(ios_props(-5, 10, 4, 'BGRA')), 0);
    check("height -5", refused(ios_props(100, -5, 4, 'BGRA')), 0);
    check("bytes per element 0", refused(ios_props(100, 10, 0, 'BGRA')), 0);
    check("an empty dictionary", refused(ios_props(-2000000, -2000000, -2000000, 0)), 0);
    CFMutableDictionaryRef d = ios_props(100, 10, 4, 'BGRA');
    ios_set_int(d, kIOSurfaceBytesPerRow, 399);
    check("BytesPerRow 399 for 100 elements of 4 bytes", refused(d), 0);
    d = ios_props(100, 10, 4, 'BGRA');
    ios_set_int(d, kIOSurfaceAllocSize, 100);
    check("AllocSize 100 for a surface that needs more", refused(d), 0);
    check("100000 by 100000", refused(ios_props(100000, 100000, 4, 'BGRA')), 0);
    d = ios_props(100, 50, 1, '420v');
    CFArrayRef none = CFArrayCreate(NULL, NULL, 0, &kCFTypeArrayCallBacks);
    CFDictionarySetValue(d, kIOSurfacePlaneInfo, none);
    check("a plane info array with no plane", refused(d), 0);
    IOSurfaceRef wide = ios_make(65536, 10, 4);
    check("a width of 65536 is made", wide != NULL && IOSurfaceGetWidth(wide) == 65536, 0);
    return finish();
}
