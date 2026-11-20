/* The header of an image is the base that dladdr reports for an address
 * inside it, and its load commands are readable (the UUID and the install
 * name of a dylib). */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <dlfcn.h>
#include <mach-o/dyld.h>
#include <mach-o/loader.h>
#include <stdlib.h>
#include <string.h>

int test_main(void) {
    uint32_t n = _dyld_image_count();
    int found_malloc = 0, found_self = 0;
    Dl_info mi, si;
    dladdr((void *)malloc, &mi);
    dladdr((void *)test_main, &si);
    for (uint32_t i = 0; i < n; i++) {
        found_malloc += _dyld_get_image_header(i) == (void *)mi.dli_fbase;
        found_self += _dyld_get_image_header(i) == (void *)si.dli_fbase;
    }
    check("the image holding malloc is in the list, by the base dladdr reports", found_malloc == 1, found_malloc);
    check("so is the program", found_self == 1 && si.dli_fbase == (void *)_dyld_get_image_header(0), found_self);

    /* Walk the load commands of libsystem_malloc and read its install name. */
    const struct mach_header_64 *h = mi.dli_fbase;
    const char *p = (const char *)(h + 1);
    int uuid = 0, idcmd = 0;
    char id[128] = "";
    for (uint32_t i = 0; i < h->ncmds; i++) {
        const struct load_command *lc = (const void *)p;
        if (lc->cmd == LC_UUID) uuid++;
        if (lc->cmd == LC_ID_DYLIB) {
            idcmd++;
            const struct dylib_command *d = (const void *)lc;
            strlcpy(id, (const char *)d + d->dylib.name.offset, sizeof id);
        }
        p += lc->cmdsize;
    }
    check("the load commands of a loaded image can be read: one LC_UUID", uuid == 1, uuid);
    check("and one LC_ID_DYLIB", idcmd == 1, idcmd);
    check_str("whose name is the image's name", id, "/usr/lib/system/libsystem_malloc.dylib");
    return finish();
}
