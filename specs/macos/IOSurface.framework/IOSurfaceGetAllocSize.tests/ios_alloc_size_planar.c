/* poly-mac: iosurface
 * The allocation of a planar surface holds each plane in whole pages: NV12 ('420v') 100 by 50 with plane 0 of 100 one-byte elements (rows of 128, 6400 bytes) and plane
 * 1 of 50 two-byte elements by 25 rows (3200 bytes) is 32768 bytes, plane 1 starting at offset 16384; with the plane offsets given (0 and 6400) and an
 * AllocSize of 16384 the planes are where the properties put them. */
#include "poly_miosurface.h"
static CFMutableDictionaryRef plane(long long w, long long h, long long bpe, long long off) {
    CFMutableDictionaryRef a = CFDictionaryCreateMutable(NULL, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    ios_set_int(a, kIOSurfacePlaneWidth, w);
    ios_set_int(a, kIOSurfacePlaneHeight, h);
    ios_set_int(a, kIOSurfacePlaneBytesPerElement, bpe);
    if (off >= 0) ios_set_int(a, kIOSurfacePlaneOffset, off);
    return a;
}
int test_main(void) {
    CFMutableDictionaryRef d = ios_props(100, 50, -2000000, '420v');
    const void *planes[2] = {plane(100, 50, 1, -1), plane(50, 25, 2, -1)};
    CFDictionarySetValue(d, kIOSurfacePlaneInfo, CFArrayCreate(NULL, planes, 2, &kCFTypeArrayCallBacks));
    IOSurfaceRef s = ios_make_props(d);
    check("NV12 100 by 50: two planes", s && IOSurfaceGetPlaneCount(s) == 2, 0);
    check("the allocation is 32768 bytes", s && IOSurfaceGetAllocSize(s) == 32768, s ? (long long)IOSurfaceGetAllocSize(s) : -1);
    check("plane 1 starts 16384 bytes after plane 0", s && (char *)IOSurfaceGetBaseAddressOfPlane(s, 1) - (char *)IOSurfaceGetBaseAddressOfPlane(s, 0) == 16384, 0);
    check("plane 0 starts at the base address", s && IOSurfaceGetBaseAddressOfPlane(s, 0) == IOSurfaceGetBaseAddress(s), 0);
    check("widths and heights of the planes as given", s && IOSurfaceGetWidthOfPlane(s, 0) == 100 && IOSurfaceGetHeightOfPlane(s, 0) == 50 && IOSurfaceGetWidthOfPlane(s, 1) == 50 && IOSurfaceGetHeightOfPlane(s, 1) == 25, 0);
    check("bytes per element of the planes: 1 and 2", s && IOSurfaceGetBytesPerElementOfPlane(s, 0) == 1 && IOSurfaceGetBytesPerElementOfPlane(s, 1) == 2, 0);
    d = ios_props(100, 50, -2000000, '420v');
    const void *pl2[2] = {plane(100, 50, 1, 0), plane(50, 25, 2, 6400)};
    CFDictionarySetValue(d, kIOSurfacePlaneInfo, CFArrayCreate(NULL, pl2, 2, &kCFTypeArrayCallBacks));
    ios_set_int(d, kIOSurfaceAllocSize, 16384);
    IOSurfaceRef t = ios_make_props(d);
    check("with plane offsets 0 and 6400 and AllocSize 16384 the planes are there", t && IOSurfaceGetAllocSize(t) == 16384 && (char *)IOSurfaceGetBaseAddressOfPlane(t, 1) - (char *)IOSurfaceGetBaseAddress(t) == 6400, 0);
    return finish();
}
