/* Several zones created, named, used and destroyed; the list holds them
 * in creation order. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <malloc/malloc.h>
#include <stdlib.h>

static unsigned count(void) {
    unsigned n = 0;
    vm_address_t *zones = NULL;
    malloc_get_all_zones(0, NULL, &zones, &n);
    return n;
}

int test_main(void) {
    unsigned n0 = count();
    malloc_zone_t *z[5];
    for (int i = 0; i < 5; i++) {
        z[i] = malloc_create_zone(0, 0);
        char nm[4] = {'z', (char)('0' + i), 0, 0};
        malloc_set_zone_name(z[i], nm);
    }
    check("five zones more", count() == n0 + 5, count());
    vm_address_t *zones;
    unsigned n;
    malloc_get_all_zones(0, NULL, &zones, &n);
    int order = 1;
    for (int i = 0; i < 5; i++) order &= (malloc_zone_t *)zones[n - 5 + i] == z[i];
    check("in creation order, at the end", order, 0);
    int distinct = 1;
    for (int i = 0; i < 5; i++)
        for (int j = i + 1; j < 5; j++) distinct &= z[i] != z[j];
    check("five different zones", distinct, 0);
    void *p[5];
    for (int i = 0; i < 5; i++) p[i] = malloc_zone_malloc(z[i], 200);
    int own = 1;
    for (int i = 0; i < 5; i++) own &= malloc_zone_from_ptr(p[i]) == z[i];
    check("each block belongs to its own zone", own, 0);
    for (int i = 0; i < 5; i++) free(p[i]);
    for (int i = 4; i >= 0; i--) malloc_destroy_zone(z[i]);
    check("destroyed in reverse order: back to the start", count() == n0, count());
    return finish();
}
