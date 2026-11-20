/* dladdr: names the image and the nearest symbol at or below an address
 * (inside the program or any loaded image); returns nonzero on success,
 * 0 and leaves the info alone for addresses in no image. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <dlfcn.h>
#include <mach-o/dyld.h>
#include <stdlib.h>
#include <string.h>
#include <pthread.h>
#include <unistd.h>

int global_var = 5;
int __attribute__((noinline)) a_function(void) { return global_var + 1; }

int test_main(void) {
    Dl_info i;
    memset(&i, 0, sizeof i);
    int r = dladdr((void *)a_function, &i);
    check("a function of the program: nonzero", r != 0, r);
    check_str("symbol name (no underscore)", i.dli_sname, "a_function");
    check("symbol address is the function", i.dli_saddr == (void *)a_function, 0);
    check("image base is the program's header", i.dli_fbase == (void *)_dyld_get_image_header(0), 0);
    check("file name is image 0's name", i.dli_fname && strcmp(i.dli_fname, _dyld_get_image_name(0)) == 0, 0);

    r = dladdr((char *)a_function + 3, &i);
    check("an address inside the function: nonzero", r != 0, r);
    check_str("the same symbol", i.dli_sname, "a_function");
    check("saddr is still the function's start", i.dli_saddr == (void *)a_function, 0);

    r = dladdr(&global_var, &i);
    check("a global variable", r != 0, r);
    check_str("its name", i.dli_sname, "global_var");

    r = dladdr((void *)malloc, &i);
    check("malloc: nonzero", r != 0, r);
    check_str("malloc's image", i.dli_fname, "/usr/lib/system/libsystem_malloc.dylib");
    check_str("malloc's symbol", i.dli_sname, "malloc");
    check("saddr is malloc", i.dli_saddr == (void *)malloc, 0);
    check("fbase is that image's header", ({ int f = 0; for (uint32_t k = 0; k < _dyld_image_count(); k++) f |= _dyld_get_image_header(k) == (void *)i.dli_fbase; f; }), 0);

    r = dladdr((void *)getpid, &i);
    check("getpid is found", r != 0, r);
    check_str("in libsystem_kernel", i.dli_fname, "/usr/lib/system/libsystem_kernel.dylib");
    check("as some symbol of that image at its start (a wrapper may be reported under an inner name)", i.dli_sname != NULL && i.dli_saddr != NULL, 0);
    value_str("symbol reported for getpid (an alias)", i.dli_sname);
    r = dladdr((void *)dlopen, &i);
    check_str("dlopen is in libdyld", i.dli_fname, "/usr/lib/system/libdyld.dylib");
    r = dladdr((void *)pthread_self, &i);
    check_str("pthread_self is in libsystem_pthread", i.dli_fname, "/usr/lib/system/libsystem_pthread.dylib");
    return finish();
}
