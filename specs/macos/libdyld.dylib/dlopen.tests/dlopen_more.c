/* dlopen: a failed call does not disturb a library opened before; the
 * error of the last call is the one dlerror returns. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <dlfcn.h>
#include <string.h>

int test_main(void) {
    void *z = dlopen("/usr/lib/libz.1.dylib", RTLD_NOW);
    void *bad1 = dlopen("/no/one.dylib", RTLD_NOW);
    void *bad2 = dlopen("/no/two.dylib", RTLD_NOW);
    char *e = dlerror();
    check("two failures in a row: the second one's message", bad1 == NULL && bad2 == NULL && e && strstr(e, "dlopen(/no/two.dylib, ") == e, 0);
    check("the first message is lost", dlerror() == NULL, 0);
    check("the library opened before still works", z && dlsym(z, "zlibVersion") != NULL, 0);
    dlopen("/no/three.dylib", RTLD_NOW);
    dlopen("/usr/lib/libz.1.dylib", RTLD_NOW);
    check("a successful dlopen clears an unread error", dlerror() == NULL, 0);
    dlopen("/no/four.dylib", RTLD_NOW);
    dlsym(RTLD_DEFAULT, "malloc");
    check("so does a successful dlsym", dlerror() == NULL, 0);
    dlopen("/no/five.dylib", RTLD_NOW);
    dlclose(z);
    check("and a dlclose", dlerror() == NULL, 0);
    return finish();
}
