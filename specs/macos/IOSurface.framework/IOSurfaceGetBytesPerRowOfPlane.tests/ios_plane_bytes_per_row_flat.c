/* poly-mac: iosurface
 * For a surface with no planes IOSurfaceGetBytesPerRowOfPlane(s, 0) is the surface's BytesPerRow and any other plane gives 0; the same for the other per-plane
 * queries (IOSurfaceGetWidthOfPlane, GetHeightOfPlane, GetBytesPerElementOfPlane): plane 0 is the surface. */
#include "poly_miosurface.h"
int test_main(void) {
    IOSurfaceRef s = ios_make(100, 50, 4);
    check("plane 0 BytesPerRow is the surface's", IOSurfaceGetBytesPerRowOfPlane(s, 0) == IOSurfaceGetBytesPerRow(s) && IOSurfaceGetBytesPerRow(s) == 512, 0);
    check("plane 1 BytesPerRow is 0", IOSurfaceGetBytesPerRowOfPlane(s, 1) == 0, 0);
    check("plane 5 BytesPerRow is 0", IOSurfaceGetBytesPerRowOfPlane(s, 5) == 0, 0);
    check("plane 0 width, height, bytes per element are the surface's", IOSurfaceGetWidthOfPlane(s, 0) == 100 && IOSurfaceGetHeightOfPlane(s, 0) == 50 && IOSurfaceGetBytesPerElementOfPlane(s, 0) == 4, 0);
    return finish();
}
