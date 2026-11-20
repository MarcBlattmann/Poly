/* _dyld_get_image_name: the path of image i; image 0 is the program by
 * the absolute path it was started with; system libraries by their install
 * names (not the dyld shared cache's); NULL when out of range. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <mach-o/dyld.h>
#include <string.h>

int test_main(void) {
    uint32_t n = _dyld_image_count();
    const char *n0 = _dyld_get_image_name(0);
    check("image 0 has a name", n0 != NULL, 0);
    check("image 0 is an absolute path", n0 && n0[0] == '/', 0);
    char exe[1024];
    uint32_t sz = sizeof exe;
    check("_NSGetExecutablePath works", _NSGetExecutablePath(exe, &sz) == 0, 0);
    check("image 0 is the path the program was started with", strcmp(n0, exe) == 0, 0);
    check_str("image 1 is libSystem", _dyld_get_image_name(1), "/usr/lib/libSystem.B.dylib");

    int bad = 0, system_paths = 0;
    for (uint32_t i = 1; i < n; i++) {
        const char *nm = _dyld_get_image_name(i);
        if (!nm || nm[0] != '/') bad++;
        else system_paths += strncmp(nm, "/usr/lib/", 9) == 0;
    }
    check("every name is an absolute path", bad == 0, bad);
    check("every other image of a libSystem-only program is under /usr/lib", system_paths == (int)n - 1, system_paths);
    check("the same string every time", _dyld_get_image_name(1) == _dyld_get_image_name(1), 0);
    check("index == count: NULL", _dyld_get_image_name(n) == NULL, 0);
    check("index -1: NULL", _dyld_get_image_name((uint32_t)-1) == NULL, 0);
    return finish();
}
