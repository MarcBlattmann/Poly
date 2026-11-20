/* malloc_default_zone: the zone malloc() uses; a table of function
 * pointers (struct malloc_zone_t, <malloc/malloc.h>); version 10, name
 * "DefaultMallocZone". */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <malloc/malloc.h>
#include <stdint.h>
#include <stdlib.h>

int test_main(void) {
    malloc_zone_t *z = malloc_default_zone();
    check("not NULL", z != NULL, 0);
    check("the same zone on every call", z == malloc_default_zone(), 0);
    check("version 10", z->version == 10, z->version);
    check_str("name", z->zone_name, "DefaultMallocZone");
    check("the usual entries are filled in", z->size && z->malloc && z->calloc && z->valloc && z->free && z->realloc && z->destroy, 0);
    check("batch_malloc, batch_free, introspect, memalign, free_definite_size, pressure_relief, claimed_address are filled in",
          z->batch_malloc && z->batch_free && z->introspect && z->memalign && z->free_definite_size && z->pressure_relief && z->claimed_address, 0);
    value("sizeof(malloc_zone_t)", sizeof(malloc_zone_t));
    value("offsetof zone_name", (long long)((char *)&z->zone_name - (char *)z));
    value("offsetof version", (long long)((char *)&z->version - (char *)z));

    void *p = malloc(100);
    check("malloc_zone_from_ptr(malloc block) is the default zone", malloc_zone_from_ptr(p) == z, 0);
    check("zone->size(p) equals malloc_size(p)", z->size(z, p) == malloc_size(p), (long long)z->size(z, p));
    check("zone->size of NULL is 0", z->size(z, NULL) == 0, 0);
    int local = 0;
    check("zone->size of a stack address is 0", z->size(z, &local) == 0, 0);
    check("malloc_zone_from_ptr(NULL) is NULL", malloc_zone_from_ptr(NULL) == NULL, 0);
    check("malloc_zone_from_ptr(stack) is NULL", malloc_zone_from_ptr(&local) == NULL, 0);
    free(p);
    check_str("malloc_get_zone_name", malloc_get_zone_name(z), "DefaultMallocZone");

    void *a = z->malloc(z, 50);
    void *b = z->realloc(z, a, 500);
    check("zone->malloc / realloc give blocks of the zone", b && malloc_size(b) >= 500, 0);
    z->free(z, b);
    void *c = z->calloc(z, 3, 7);
    int zero = 1;
    for (int i = 0; i < 21; i++) zero &= ((char *)c)[i] == 0;
    check("zone->calloc zero-fills", zero, 0);
    free(c);
    void *v = z->valloc(z, 10);
    check("zone->valloc is page aligned", ((uintptr_t)v & 4095) == 0, 0);
    free(v);
    void *m = z->memalign(z, 256, 10);
    check("zone->memalign honours the alignment", ((uintptr_t)m & 255) == 0, 0);
    free(m);
    return finish();
}
