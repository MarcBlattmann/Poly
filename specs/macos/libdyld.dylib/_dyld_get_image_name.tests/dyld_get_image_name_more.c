/* The name of an image loaded later is its resolved path or the name it
 * was loaded by: dlopen through a symbolic link. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <dlfcn.h>
#include <mach-o/dyld.h>
#include <string.h>

static int count_name(const char *name) {
    int c = 0;
    for (uint32_t i = 0; i < _dyld_image_count(); i++) c += strcmp(_dyld_get_image_name(i), name) == 0;
    return c;
}

int test_main(void) {
    void *h = dlopen("/usr/lib/libz.dylib", RTLD_NOW);
    check("dlopen via the libz.dylib link", h != NULL, 0);
    check("the image is listed under its real name libz.1.dylib", count_name("/usr/lib/libz.1.dylib") == 1, count_name("/usr/lib/libz.1.dylib"));
    check("and not under the link's name", count_name("/usr/lib/libz.dylib") == 0, count_name("/usr/lib/libz.dylib"));
    return finish();
}
