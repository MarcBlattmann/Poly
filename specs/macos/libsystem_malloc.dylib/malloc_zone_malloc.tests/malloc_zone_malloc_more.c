/* Many blocks from two zones, freed in a different order. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <malloc/malloc.h>
#include <stdlib.h>
#include <string.h>

int test_main(void) {
    malloc_zone_t *a = malloc_default_zone(), *b = malloc_create_zone(0, 0);
    void *pa[500], *pb[500];
    for (int i = 0; i < 500; i++) {
        pa[i] = malloc_zone_malloc(a, (size_t)(i * 13 % 3000));
        pb[i] = malloc_zone_malloc(b, (size_t)(i * 17 % 3000));
        memset(pa[i], i, malloc_size(pa[i]));
        memset(pb[i], ~i, malloc_size(pb[i]));
    }
    int bad = 0;
    for (int i = 0; i < 500; i++) {
        bad += malloc_zone_from_ptr(pa[i]) != a || malloc_zone_from_ptr(pb[i]) != b;
        bad += ((unsigned char *)pa[i])[0] != (unsigned char)i || ((unsigned char *)pb[i])[0] != (unsigned char)~i;
    }
    check("1000 blocks of two zones keep their owner and their content", bad == 0, bad);
    for (int i = 499; i >= 0; i--) {
        free(pa[i]);
        malloc_zone_free(b, pb[i]);
    }
    check("freed through free() and malloc_zone_free in reverse order", 1, 0);
    return finish();
}
