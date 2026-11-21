/* poly-mac: iosurface
 * The other properties are not aligned: IOSurfaceAlignProperty of kIOSurfaceWidth, kIOSurfaceHeight, kIOSurfaceBytesPerElement and kIOSurfaceAllocSize gives the
 * value back (16385 stays 16385), and IOSurfaceGetPropertyAlignment of each of them is 1. IOSurfaceGetPropertyMaximum is 16384 for the width and the height, 32768 for the
 * BytesPerRow and 0 (no maximum) for the element size and the allocation. */
#include "poly_miosurface.h"
int test_main(void) {
    CFStringRef keys[] = {kIOSurfaceWidth, kIOSurfaceHeight, kIOSurfaceBytesPerElement, kIOSurfaceAllocSize};
    const char *names[] = {"width", "height", "bytes per element", "allocation size"};
    for (int i = 0; i < 4; i++) {
        char name[64];
        snprintf(name, sizeof name, "%s: 16385 stays 16385 and the alignment is 1", names[i]);
        check(name, IOSurfaceAlignProperty(keys[i], 16385) == 16385 && IOSurfaceGetPropertyAlignment(keys[i]) == 1, (long long)IOSurfaceAlignProperty(keys[i], 16385));
    }
    check("maximum width 16384", IOSurfaceGetPropertyMaximum(kIOSurfaceWidth) == 16384, (long long)IOSurfaceGetPropertyMaximum(kIOSurfaceWidth));
    check("maximum height 16384", IOSurfaceGetPropertyMaximum(kIOSurfaceHeight) == 16384, (long long)IOSurfaceGetPropertyMaximum(kIOSurfaceHeight));
    check("maximum BytesPerRow 32768", IOSurfaceGetPropertyMaximum(kIOSurfaceBytesPerRow) == 32768, (long long)IOSurfaceGetPropertyMaximum(kIOSurfaceBytesPerRow));
    check("no maximum for the allocation size", IOSurfaceGetPropertyMaximum(kIOSurfaceAllocSize) == 0, 0);
    value("maximum of bytes per element", (long long)IOSurfaceGetPropertyMaximum(kIOSurfaceBytesPerElement));
    return finish();
}
