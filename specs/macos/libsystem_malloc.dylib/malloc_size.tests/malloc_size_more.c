/* malloc_size over every request 0..4096: monotonic, multiple of 16, and
 * equal to malloc_good_size. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <malloc/malloc.h>
#include <stdlib.h>

int test_main(void) {
    int bad = 0, nonmono = 0, disagree = 0;
    size_t prev = 0;
    for (size_t n = 0; n <= 4096; n++) {
        void *p = malloc(n);
        size_t s = malloc_size(p);
        bad += s < n || s % 16 || s == 0;
        nonmono += s < prev;
        disagree += s != malloc_good_size(n);
        prev = s;
        free(p);
    }
    check("every request 0..4096: size >= request, multiple of 16", bad == 0, bad);
    check("sizes never decrease as the request grows", nonmono == 0, nonmono);
    check("malloc_size(malloc(n)) == malloc_good_size(n)", disagree == 0, disagree);
    return finish();
}
