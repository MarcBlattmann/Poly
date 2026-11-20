/* dlsym error strings: the handle shown is its address in hex or the
 * RTLD_ name; the reason is "symbol not found" or "invalid handle". */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <dlfcn.h>
#include <string.h>

static int starts(const char *s, const char *p) { return s && strncmp(s, p, strlen(p)) == 0; }

int test_main(void) {
    void *z = dlopen("/usr/lib/libz.1.dylib", RTLD_NOW);
    check("no error to start", dlerror() == NULL, 0);
    void *s = dlsym(z, "no_such_symbol");
    char *e = dlerror();
    check("a missing symbol: NULL", s == NULL, 0);
    check("message: dlsym(0x..., name): symbol not found", starts(e, "dlsym(0x") && strstr(e, ", no_such_symbol): symbol not found") != NULL, 0);
    check("the message is cleared by reading", dlerror() == NULL, 0);

    s = dlsym(RTLD_DEFAULT, "nope_nope");
    e = dlerror();
    check_str("RTLD_DEFAULT is named", e, "dlsym(RTLD_DEFAULT, nope_nope): symbol not found");
    s = dlsym(RTLD_NEXT, "nope_nope");
    e = dlerror();
    check_str("RTLD_NEXT is named", e, "dlsym(RTLD_NEXT, nope_nope): symbol not found");
    s = dlsym(RTLD_SELF, "nope_nope");
    e = dlerror();
    check_str("RTLD_SELF is named", e, "dlsym(RTLD_SELF, nope_nope): symbol not found");
    s = dlsym(RTLD_MAIN_ONLY, "nope_nope");
    e = dlerror();
    check_str("RTLD_MAIN_ONLY is named", e, "dlsym(RTLD_MAIN_ONLY, nope_nope): symbol not found");

    s = dlsym((void *)0x1234, "x");
    e = dlerror();
    check("a handle that was never given out: NULL", s == NULL, 0);
    check_str("is an invalid handle, shown in hex", e, "dlsym(0x1234, x): invalid handle");
    s = dlsym(NULL, "malloc");
    e = dlerror();
    check("a NULL handle is not RTLD_DEFAULT: NULL", s == NULL, 0);
    check_str("and is an invalid handle too", e, "dlsym(0x0, malloc): invalid handle");

    dlsym(z, "a");
    dlsym(z, "b");
    e = dlerror();
    check("two failures in a row: dlerror holds the last", e && strstr(e, ", b): symbol not found"), 0);
    return finish();
}
