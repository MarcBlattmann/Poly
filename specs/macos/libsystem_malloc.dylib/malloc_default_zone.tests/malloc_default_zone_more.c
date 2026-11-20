/* The default zone's table agrees with malloc, calloc, realloc. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <malloc/malloc.h>
#include <stdlib.h>
#include <string.h>

int test_main(void) {
    malloc_zone_t *z = malloc_default_zone();
    int bad = 0;
    for (size_t n = 0; n < 2000; n += 37) {
        void *p = malloc(n);
        bad += z->size(z, p) != malloc_size(p);
        bad += malloc_zone_from_ptr(p) != z;
        void *q = z->malloc(z, n);
        bad += z->size(z, q) != z->size(z, p);
        free(p);
        z->free(z, q);
    }
    check("zone and global functions agree for 55 sizes", bad == 0, bad);
    char *s = z->malloc(z, 10);
    strcpy(s, "012345678");
    s = z->realloc(z, s, 100000);
    check("realloc through the table keeps the contents", strcmp(s, "012345678") == 0, 0);
    free(s);
    return finish();
}
