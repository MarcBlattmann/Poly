/* poly-mac: iosurface
 * IOSurfaceAlignProperty(kIOSurfaceBytesPerRow, n) rounds n up to a multiple of 128: 100 gives 128, 128 gives 128, 129 gives 256, 401 gives 512, 0 gives 0;
 * the value it gives for a width * bytes per element is the BytesPerRow a surface of that width gets. IOSurfaceGetPropertyAlignment(kIOSurfaceBytesPerRow)
 * is 128. */
#include "poly_miosurface.h"
int test_main(void) {
    check("100 gives 128", IOSurfaceAlignProperty(kIOSurfaceBytesPerRow, 100) == 128, IOSurfaceAlignProperty(kIOSurfaceBytesPerRow, 100));
    check("128 gives 128", IOSurfaceAlignProperty(kIOSurfaceBytesPerRow, 128) == 128, 0);
    check("129 gives 256", IOSurfaceAlignProperty(kIOSurfaceBytesPerRow, 129) == 256, 0);
    check("401 gives 512", IOSurfaceAlignProperty(kIOSurfaceBytesPerRow, 401) == 512, 0);
    check("1 gives 128", IOSurfaceAlignProperty(kIOSurfaceBytesPerRow, 1) == 128, 0);
    value("0 gives", (long long)IOSurfaceAlignProperty(kIOSurfaceBytesPerRow, 0));
    check("the alignment of BytesPerRow is 128", IOSurfaceGetPropertyAlignment(kIOSurfaceBytesPerRow) == 128, IOSurfaceGetPropertyAlignment(kIOSurfaceBytesPerRow));
    for (int w = 1; w < 400; w += 37) {
        IOSurfaceRef s = ios_make(w, 2, 4);
        if (!s || IOSurfaceGetBytesPerRow(s) != IOSurfaceAlignProperty(kIOSurfaceBytesPerRow, (size_t)w * 4)) {
            check("a surface's BytesPerRow is the aligned width * bytes per element", 0, w);
            return finish();
        }
    }
    check("a surface's BytesPerRow is the aligned width * bytes per element", 1, 0);
    return finish();
}
