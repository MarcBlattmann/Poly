/* _dyld_get_image_header: the mach_header_64 of image i; image 0 is the
 * program (MH_EXECUTE), the others MH_DYLIB; NULL when the index is out of
 * range. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <dlfcn.h>
#include <mach-o/dyld.h>
#include <mach-o/loader.h>
#include <stdint.h>

extern const struct mach_header_64 _mh_execute_header;

int test_main(void) {
    uint32_t n = _dyld_image_count();
    const struct mach_header_64 *h0 = (const void *)_dyld_get_image_header(0);
    check("image 0 has a header", h0 != NULL, 0);
    check("magic MH_MAGIC_64 (0xfeedfacf)", h0->magic == MH_MAGIC_64, h0->magic);
    check("it is the program's own header", h0 == &_mh_execute_header, 0);
    check("file type MH_EXECUTE (2)", h0->filetype == MH_EXECUTE, h0->filetype);
    value("cputype (x86_64 = 0x01000007, arm64 = 0x0100000c)", h0->cputype);

    int bad = 0, dylibs = 0;
    for (uint32_t i = 1; i < n; i++) {
        const struct mach_header_64 *h = (const void *)_dyld_get_image_header(i);
        if (!h || h->magic != MH_MAGIC_64 || h->cputype != h0->cputype) bad++;
        dylibs += h && h->filetype == MH_DYLIB;
    }
    check("every image has a header with the same magic and cpu type", bad == 0, bad);
    check("all other images are MH_DYLIB (6)", dylibs == (int)n - 1, dylibs);

    check("index == count: NULL", _dyld_get_image_header(n) == NULL, 0);
    check("a huge index: NULL", _dyld_get_image_header(0x7fffffff) == NULL, 0);
    check("index -1: NULL", _dyld_get_image_header((uint32_t)-1) == NULL, 0);
    return finish();
}
