/* __cxa_atexit(func, arg, dso): func(arg) runs at exit, in reverse order of
 * registration, mixed with atexit handlers in one list. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <stdlib.h>

extern int __cxa_atexit(void (*)(void *), void *, void *);
extern void *__dso_handle;

static void with_arg(void *a) {
    put("cxa handler, arg ");
    put((const char *)a);
    put("\n");
}
static void plain(void) { put("atexit handler\n"); }

int test_main(void) {
    check("__cxa_atexit returns 0", __cxa_atexit(with_arg, (void *)"A", &__dso_handle) == 0, 0);
    check("atexit returns 0", atexit(plain) == 0, 0);
    check("__cxa_atexit again", __cxa_atexit(with_arg, (void *)"B", &__dso_handle) == 0, 0);
    check("a NULL argument is passed on (checked by the handler running)", __cxa_atexit(with_arg, (void *)"C", NULL) == 0, 0);
    put("registered A, plain, B, C; main returns\n");
    return finish();
}
