/* poly-mac: iosurface
 * IOSurfaceGetUseCount of a surface made by a surface function that was released and made again is 0 (the count is not kept in the properties: IOSurfaceCopyValue of
 * a use count key is NULL), and the use count properties kIOSurfaceIsGlobal and the like do not appear in the surface's attachments. */
#include "poly_miosurface.h"
int test_main(void) {
    IOSurfaceRef s = ios_make(32, 32, 4);
    IOSurfaceIncrementUseCount(s);
    check("count 1", IOSurfaceGetUseCount(s) == 1, 0);
    check("no attachment says so", IOSurfaceCopyValue(s, ios_cfstr("IOSurfaceUseCount")) == NULL && IOSurfaceCopyValue(s, ios_cfstr("UseCount")) == NULL, 0);
    IOSurfaceRef t = ios_make(32, 32, 4);
    check("a new surface after it has count 0", IOSurfaceGetUseCount(t) == 0, 0);
    return finish();
}
