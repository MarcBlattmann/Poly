/* Image 0 is the program and 1 is libSystem; a dlopen of a library that is
 * already loaded adds nothing. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <dlfcn.h>
#include <mach-o/dyld.h>
#include <string.h>

int test_main(void) {
    check_str("image 1 is libSystem", _dyld_get_image_name(1), "/usr/lib/libSystem.B.dylib");
    uint32_t n = _dyld_image_count();
    void *h = dlopen("/usr/lib/libSystem.B.dylib", RTLD_NOW);
    check("dlopen of an image that is loaded", h != NULL, 0);
    check("the count is unchanged", _dyld_image_count() == n, _dyld_image_count());
    h = dlopen("/usr/lib/system/libsystem_c.dylib", RTLD_NOW);
    check("dlopen of a sub-library of libSystem: count unchanged", _dyld_image_count() == n, _dyld_image_count());
    return finish();
}
