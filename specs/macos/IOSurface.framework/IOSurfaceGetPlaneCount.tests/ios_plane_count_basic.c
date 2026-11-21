/* poly-mac: iosurface
 * IOSurfaceGetPlaneCount is 0 for a surface made without plane information, also for a pixel format that is planar ('420v' with bytes per element and no planes);
 * the number of entries of kIOSurfacePlaneInfo (1 or 2 here) otherwise; with 2 entries IOSurfaceGetWidthOfPlane and GetHeightOfPlane are those of the entries. */
#include "poly_miosurface.h"
static CFDictionaryRef plane(long long w, long long h, long long bpe) {
    CFMutableDictionaryRef a = CFDictionaryCreateMutable(NULL, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    ios_set_int(a, kIOSurfacePlaneWidth, w);
    ios_set_int(a, kIOSurfacePlaneHeight, h);
    ios_set_int(a, kIOSurfacePlaneBytesPerElement, bpe);
    return a;
}
static IOSurfaceRef planar(int n, long long w, long long h) {
    CFMutableDictionaryRef d = ios_props(w, h, -2000000, '420v');
    const void *planes[2] = {plane(w, h, 1), plane(w / 2, h / 2, 2)};
    CFDictionarySetValue(d, kIOSurfacePlaneInfo, CFArrayCreate(NULL, planes, n, &kCFTypeArrayCallBacks));
    return ios_make_props(d);
}
int test_main(void) {
    IOSurfaceRef flat = ios_make(100, 50, 4);
    check("a plain surface: 0 planes", IOSurfaceGetPlaneCount(flat) == 0, 0);
    CFMutableDictionaryRef d = ios_props(100, 50, 1, '420v');
    IOSurfaceRef noplanes = ios_make_props(d);
    check("'420v' with bytes per element and no plane info: 0 planes", noplanes && IOSurfaceGetPlaneCount(noplanes) == 0, 0);
    IOSurfaceRef two = planar(2, 100, 50);
    check("two plane entries: 2 planes", two && IOSurfaceGetPlaneCount(two) == 2, 0);
    check("the widths and heights are the entries'", two && IOSurfaceGetWidthOfPlane(two, 0) == 100 && IOSurfaceGetHeightOfPlane(two, 0) == 50 && IOSurfaceGetWidthOfPlane(two, 1) == 50 && IOSurfaceGetHeightOfPlane(two, 1) == 25, 0);
    IOSurfaceRef one = planar(1, 100, 50);
    check("one plane entry: 1 plane", one && IOSurfaceGetPlaneCount(one) == 1, 0);
    IOSurfaceRef odd = planar(2, 101, 51);
    check("an odd size: plane 1 is the entry given, 50 wide", odd && IOSurfaceGetWidthOfPlane(odd, 1) == 50, 0);
    return finish();
}
