/* poly-mac: iosurface
 * For a non-planar surface IOSurfaceGetBytesPerRowOfPlane(s, 0) is the BytesPerRow; for a planar one IOSurfaceGetBytesPerRow is that of plane 0. */
#include "poly_miosurface.h"
static IOSurfaceRef nv12(int w, int h) {
    CFMutableDictionaryRef d = ios_props(w, h, -2000000, '420v');
    CFMutableDictionaryRef p0 = ios_props(w, h, 1, 0), p1 = ios_props(w / 2, h / 2, 2, 0);
    CFDictionaryRef q0 = CFDictionaryCreateMutable(NULL, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    (void)q0;
    CFMutableDictionaryRef a = CFDictionaryCreateMutable(NULL, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    ios_set_int(a, kIOSurfacePlaneWidth, w);
    ios_set_int(a, kIOSurfacePlaneHeight, h);
    ios_set_int(a, kIOSurfacePlaneBytesPerElement, 1);
    CFMutableDictionaryRef b = CFDictionaryCreateMutable(NULL, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    ios_set_int(b, kIOSurfacePlaneWidth, w / 2);
    ios_set_int(b, kIOSurfacePlaneHeight, h / 2);
    ios_set_int(b, kIOSurfacePlaneBytesPerElement, 2);
    const void *planes[2] = {a, b};
    CFArrayRef arr = CFArrayCreate(NULL, planes, 2, &kCFTypeArrayCallBacks);
    CFDictionarySetValue(d, kIOSurfacePlaneInfo, arr);
    CFRelease(p0);
    CFRelease(p1);
    return ios_make_props(d);
}
int test_main(void) {
    IOSurfaceRef flat = ios_make(100, 50, 4);
    check("non-planar: plane 0 is the whole: BytesPerRowOfPlane(0) is the BytesPerRow", IOSurfaceGetBytesPerRowOfPlane(flat, 0) == IOSurfaceGetBytesPerRow(flat), IOSurfaceGetBytesPerRowOfPlane(flat, 0));
    IOSurfaceRef s = nv12(100, 50);
    check("NV12 100 by 50: two planes", s && IOSurfaceGetPlaneCount(s) == 2, 0);
    check("plane 0: 100 one-byte elements per row, aligned to 128", IOSurfaceGetBytesPerRowOfPlane(s, 0) == 128, IOSurfaceGetBytesPerRowOfPlane(s, 0));
    check("plane 1: 50 two-byte elements per row, aligned to 128", IOSurfaceGetBytesPerRowOfPlane(s, 1) == 128, IOSurfaceGetBytesPerRowOfPlane(s, 1));
    check("IOSurfaceGetBytesPerRow is plane 0's", IOSurfaceGetBytesPerRow(s) == IOSurfaceGetBytesPerRowOfPlane(s, 0), 0);
    check("a plane that does not exist: 0", IOSurfaceGetBytesPerRowOfPlane(s, 2) == 0 && IOSurfaceGetBytesPerRowOfPlane(flat, 1) == 0, 0);
    return finish();
}
