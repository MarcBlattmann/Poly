/* poly-mac: iosurface
 * A surface made with kIOSurfaceIsGlobal true can be found by its ID in another process (IOSurfaceLookup there gives its memory and sizes); one made without cannot
 * (NULL there). */
#include "poly_miosurface.h"
int test_main(void) {
    if (is_child("child")) {
        unsigned id = (unsigned)strtoul(ios_child_arg(), NULL, 10);
        IOSurfaceRef s = IOSurfaceLookup(id);
        int global = poly_argc > 3 && poly_argv[3][0] == 'g';
        if (global) {
            check("child: a global surface is found by its ID", s != NULL, 0);
            if (s) {
                IOSurfaceLock(s, kIOSurfaceLockReadOnly, NULL);
                check("child: with its width and its data", IOSurfaceGetWidth(s) == 64 && ((unsigned char *)IOSurfaceGetBaseAddress(s))[0] == 0x42, 0);
                IOSurfaceUnlock(s, kIOSurfaceLockReadOnly, NULL);
            }
        } else {
            check("child: a surface that is not global is not found: NULL", s == NULL, 0);
        }
        return finish();
    }
    for (int g = 0; g < 2; g++) {
        CFMutableDictionaryRef d = ios_props(64, 64, 4, 'BGRA');
        if (g) CFDictionarySetValue(d, kIOSurfaceIsGlobal, kCFBooleanTrue);
        IOSurfaceRef s = ios_make_props(d);
        IOSurfaceLock(s, 0, NULL);
        ((unsigned char *)IOSurfaceGetBaseAddress(s))[0] = 0x42;
        IOSurfaceUnlock(s, 0, NULL);
        char id[24];
        snprintf(id, sizeof id, "%u", (unsigned)IOSurfaceGetID(s));
        char *argv[5] = {poly_argv[0], "child", id, g ? "g" : "n", NULL};
        pid_t pid = -1;
        posix_spawn(&pid, poly_argv[0], NULL, NULL, argv, environ);
        check(g ? "the child of the global surface ended with 0" : "the child of the other surface ended with 0", ios_wait(pid) == 0, 0);
    }
    return finish();
}
