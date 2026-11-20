/* poly-mac: iosurface
 * IOSurfaceCreate(properties) with a width, a height, bytes per element and a pixel format makes a surface that reports them back (IOSurfaceGetWidth, GetHeight,
 * GetBytesPerElement, GetPixelFormat); a surface made without a pixel format reports 0, one made without bytes per element reports 1; the element is 1 by 1, a
 * non-planar surface has 0 planes, the seed and the use count start at 0, the ID is not 0; each call makes a new surface. */
#include "poly_miosurface.h"
int test_main(void) {
    CFMutableDictionaryRef d = ios_props(100, 50, 4, 'BGRA');
    IOSurfaceRef s = IOSurfaceCreate(d);
    CFRelease(d);
    check("IOSurfaceCreate makes a surface", s != NULL, 0);
    check("width 100", IOSurfaceGetWidth(s) == 100, IOSurfaceGetWidth(s));
    check("height 50", IOSurfaceGetHeight(s) == 50, IOSurfaceGetHeight(s));
    check("bytes per element 4", IOSurfaceGetBytesPerElement(s) == 4, IOSurfaceGetBytesPerElement(s));
    check("pixel format 'BGRA'", IOSurfaceGetPixelFormat(s) == 'BGRA', IOSurfaceGetPixelFormat(s));
    check("an element is 1 by 1", IOSurfaceGetElementWidth(s) == 1 && IOSurfaceGetElementHeight(s) == 1, 0);
    check("no planes", IOSurfaceGetPlaneCount(s) == 0, IOSurfaceGetPlaneCount(s));
    check("the seed is 0", IOSurfaceGetSeed(s) == 0, IOSurfaceGetSeed(s));
    check("the use count is 0 and the surface is not in use", IOSurfaceGetUseCount(s) == 0 && !IOSurfaceIsInUse(s), 0);
    check("the ID is not 0", IOSurfaceGetID(s) != 0, 0);
    IOSurfaceRef t = ios_make(100, 50, 4);
    check("another call makes another surface", t != NULL && t != s && IOSurfaceGetID(t) != IOSurfaceGetID(s), 0);
    CFMutableDictionaryRef d2 = ios_props(100, 50, 4, 0);
    IOSurfaceRef u = IOSurfaceCreate(d2);
    CFRelease(d2);
    check("without a pixel format the format is 0", u != NULL && IOSurfaceGetPixelFormat(u) == 0, 0);
    CFMutableDictionaryRef d3 = ios_props(100, 50, -2000000, 'BGRA');
    IOSurfaceRef v = IOSurfaceCreate(d3);
    CFRelease(d3);
    check("without bytes per element the surface has 1", v != NULL && IOSurfaceGetBytesPerElement(v) == 1, v ? (long long)IOSurfaceGetBytesPerElement(v) : -1);
    return finish();
}
