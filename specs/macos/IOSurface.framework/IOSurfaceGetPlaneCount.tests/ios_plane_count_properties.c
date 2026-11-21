/* poly-mac: iosurface
 * The properties of the planes of NV12 ('420v') 100 by 50: IOSurfaceGetBytesPerElementOfPlane 1 and 2, IOSurfaceGetBytesPerRowOfPlane 128 and 128, the width and height
 * of the surface are plane 0's, the pixel format is '420v' (0x34323076) and the surface's BytesPerElement is plane 0's. */
#include "poly_miosurface.h"
int test_main(void) {
    CFMutableDictionaryRef d = ios_props(100, 50, -2000000, '420v');
    CFMutableDictionaryRef a = CFDictionaryCreateMutable(NULL, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    ios_set_int(a, kIOSurfacePlaneWidth, 100);
    ios_set_int(a, kIOSurfacePlaneHeight, 50);
    ios_set_int(a, kIOSurfacePlaneBytesPerElement, 1);
    CFMutableDictionaryRef b = CFDictionaryCreateMutable(NULL, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    ios_set_int(b, kIOSurfacePlaneWidth, 50);
    ios_set_int(b, kIOSurfacePlaneHeight, 25);
    ios_set_int(b, kIOSurfacePlaneBytesPerElement, 2);
    const void *planes[2] = {a, b};
    CFDictionarySetValue(d, kIOSurfacePlaneInfo, CFArrayCreate(NULL, planes, 2, &kCFTypeArrayCallBacks));
    IOSurfaceRef s = ios_make_props(d);
    check("two planes", s && IOSurfaceGetPlaneCount(s) == 2, 0);
    check("bytes per element of the planes: 1 and 2", IOSurfaceGetBytesPerElementOfPlane(s, 0) == 1 && IOSurfaceGetBytesPerElementOfPlane(s, 1) == 2, 0);
    check("bytes per row of the planes: 128 and 128", IOSurfaceGetBytesPerRowOfPlane(s, 0) == 128 && IOSurfaceGetBytesPerRowOfPlane(s, 1) == 128, 0);
    check("the surface's width and height are plane 0's", IOSurfaceGetWidth(s) == 100 && IOSurfaceGetHeight(s) == 50, 0);
    check("the pixel format is '420v'", IOSurfaceGetPixelFormat(s) == 0x34323076, IOSurfaceGetPixelFormat(s));
    check("the surface's bytes per element is plane 0's: 1", IOSurfaceGetBytesPerElement(s) == 1, IOSurfaceGetBytesPerElement(s));
    check("plane 2 does not exist: 0 for its sizes", IOSurfaceGetWidthOfPlane(s, 2) == 0 && IOSurfaceGetBytesPerElementOfPlane(s, 2) == 0, 0);
    return finish();
}
