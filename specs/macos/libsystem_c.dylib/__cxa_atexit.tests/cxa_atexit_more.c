/* __cxa_atexit: the order is shared with atexit and a handler registered
 * with __cxa_atexit while handlers run is run as well. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <stdlib.h>

extern int __cxa_atexit(void (*)(void *), void *, void *);
extern void *__dso_handle;

static void tail(void *a) { put((const char *)a); put(" (registered during exit)\n"); }
static void head(void *a) {
    put((const char *)a);
    put(" runs and registers another\n");
    __cxa_atexit(tail, (void *)"tail", &__dso_handle);
}
static void mid(void) { put("mid (atexit)\n"); }

int test_main(void) {
    __cxa_atexit(head, (void *)"head", &__dso_handle);
    atexit(mid);
    __cxa_atexit(head, (void *)"head 2", &__dso_handle);
    return finish();
}
