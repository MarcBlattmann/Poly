/* _dyld_image_count after loading a framework with many dependencies, and
 * the indexes of the old images stay as they were. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <dlfcn.h>
#include <mach-o/dyld.h>
#include <string.h>

int test_main(void) {
    uint32_t n = _dyld_image_count();
    char first[8][128];
    for (int i = 0; i < 8; i++) strlcpy(first[i], _dyld_get_image_name((uint32_t)i), sizeof first[i]);
    void *h = dlopen("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation", RTLD_NOW);
    check("dlopen of CoreFoundation", h != NULL, 0);
    uint32_t after = _dyld_image_count();
    check("CoreFoundation and its dependencies add images", after > n, (long long)after - n);
    int same = 1;
    for (int i = 0; i < 8; i++) same &= strcmp(first[i], _dyld_get_image_name((uint32_t)i)) == 0;
    check("the first 8 images keep their index and name", same, 0);
    int found = 0;
    for (uint32_t i = n; i < after; i++) found += strstr(_dyld_get_image_name(i), "CoreFoundation") != NULL;
    check("a new image has CoreFoundation in its name, and new images come at the end", found >= 1, found);
    return finish();
}
