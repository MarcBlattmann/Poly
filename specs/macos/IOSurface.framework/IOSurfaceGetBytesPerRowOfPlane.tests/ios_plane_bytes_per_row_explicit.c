/* poly-mac: iosurface
 * A kIOSurfacePlaneBytesPerRow in the plane information is kept (here 400 and 300 for planes of 100 and 50 elements) and IOSurfaceGetBytesPerRowOfPlane reports it. */
#include "poly_miosurface.h"
static CFMutableDictionaryRef plane(long long w, long long h, long long bpe, long long bpr) {
    CFMutableDictionaryRef a = CFDictionaryCreateMutable(NULL, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    ios_set_int(a, kIOSurfacePlaneWidth, w);
    ios_set_int(a, kIOSurfacePlaneHeight, h);
    ios_set_int(a, kIOSurfacePlaneBytesPerElement, bpe);
    ios_set_int(a, kIOSurfacePlaneBytesPerRow, bpr);
    return a;
}
int test_main(void) {
    CFMutableDictionaryRef d = ios_props(100, 50, -2000000, '420v');
    const void *planes[2] = {plane(100, 50, 1, 400), plane(50, 25, 2, 300)};
    CFDictionarySetValue(d, kIOSurfacePlaneInfo, CFArrayCreate(NULL, planes, 2, &kCFTypeArrayCallBacks));
    IOSurfaceRef s = ios_make_props(d);
    check("the surface is made", s != NULL, 0);
    check("plane 0 keeps 400", s && IOSurfaceGetBytesPerRowOfPlane(s, 0) == 400, s ? (long long)IOSurfaceGetBytesPerRowOfPlane(s, 0) : -1);
    check("plane 1 keeps 300", s && IOSurfaceGetBytesPerRowOfPlane(s, 1) == 300, s ? (long long)IOSurfaceGetBytesPerRowOfPlane(s, 1) : -1);
    value("the surface's own BytesPerRow", s ? (long long)IOSurfaceGetBytesPerRow(s) : -1);
    return finish();
}
