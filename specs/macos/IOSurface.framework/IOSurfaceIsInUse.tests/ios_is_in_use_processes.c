/* poly-mac: iosurface
 * The use count is the calling process's own and IOSurfaceIsInUse is true when any process has a use count above 0: with the child holding the surface (two increments) the parent
 * sees its own count 0 and the surface in use; when the child has dropped one of its two the surface is still in use; when the child exits its counts are dropped
 * with it and the surface is not in use any more. */
#include "poly_miosurface.h"
int test_main(void) {
    if (is_child("child")) {
        unsigned id = (unsigned)strtoul(ios_child_arg(), NULL, 10);
        IOSurfaceRef s = IOSurfaceLookup(id);
        if (!s) return 2;
        IOSurfaceIncrementUseCount(s);
        IOSurfaceIncrementUseCount(s);
        check("child: its use count is 2", IOSurfaceGetUseCount(s) == 2, IOSurfaceGetUseCount(s));
        usleep(1200000);
        IOSurfaceDecrementUseCount(s);
        usleep(1000000);
        return finish();
    }
    CFMutableDictionaryRef d = ios_props(32, 32, 4, 'BGRA');
    CFDictionarySetValue(d, kIOSurfaceIsGlobal, kCFBooleanTrue);
    IOSurfaceRef s = ios_make_props(d);
    char id[24];
    snprintf(id, sizeof id, "%u", (unsigned)IOSurfaceGetID(s));
    pid_t pid = ios_start_self("child", id);
    usleep(700000);
    check("the child holds it: the parent's own count is 0", IOSurfaceGetUseCount(s) == 0, IOSurfaceGetUseCount(s));
    check("and the surface is in use", IOSurfaceIsInUse(s), 0);
    usleep(1000000);
    check("after the child dropped one of two: still in use", IOSurfaceIsInUse(s), 0);
    int status = ios_wait(pid);
    check("the child ended with status 0", status == 0, status);
    check("after it exited: not in use", !IOSurfaceIsInUse(s), 0);
    return finish();
}
