/* malloc_create_zone / malloc_set_zone_name / malloc_get_zone_name /
 * malloc_zone_register / malloc_zone_unregister / malloc_get_all_zones /
 * malloc_destroy_zone. The number of zones that exist at start differs
 * between the architectures (x86_64 has an extra "MallocHelperZone"), so
 * only changes in the count are checked. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <malloc/malloc.h>
#include <stdlib.h>
#include <string.h>

static unsigned count(void) {
    unsigned n = 0;
    vm_address_t *zones = NULL;
    if (malloc_get_all_zones(0, NULL, &zones, &n) != 0) return 9999;
    return n;
}
static int listed(malloc_zone_t *z) {
    unsigned n = 0;
    vm_address_t *zones = NULL;
    malloc_get_all_zones(0, NULL, &zones, &n);
    for (unsigned i = 0; i < n; i++)
        if ((malloc_zone_t *)zones[i] == z) return 1;
    return 0;
}

int test_main(void) {
    unsigned n0 = count();
    value("zones at start (includes the default zone)", n0);
    check("there are zones", n0 >= 2 && n0 < 10, n0);
    vm_address_t *zones = NULL;
    unsigned n = 0;
    malloc_get_all_zones(0, NULL, &zones, &n);
    check_str("the first zone is the default zone", malloc_get_zone_name((malloc_zone_t *)zones[0]), "DefaultMallocZone");

    malloc_zone_t *z = malloc_create_zone(0, 0);
    check("create returns a zone", z != NULL, 0);
    check("a new zone has no name", malloc_get_zone_name(z) == NULL, 0);
    check("and version 16", z->version == 16, z->version);
    check("it is registered at once: the count grew by 1", count() == n0 + 1, count());
    check("and listed, last", listed(z), 0);
    malloc_get_all_zones(0, NULL, &zones, &n);
    check("the new zone is the last in the list", (malloc_zone_t *)zones[n - 1] == z, 0);
    malloc_set_zone_name(z, "poly-zone");
    check_str("set_zone_name then get_zone_name", malloc_get_zone_name(z), "poly-zone");
    char name[16] = "temp-name";
    malloc_set_zone_name(z, name);
    strcpy(name, "XXXXXXXXX");
    check_str("the name is copied, not kept as a pointer", malloc_get_zone_name(z), "temp-name");
    malloc_set_zone_name(z, NULL);
    check("a NULL name clears it", malloc_get_zone_name(z) == NULL, 0);
    malloc_set_zone_name(z, "poly-zone");

    void *p = malloc_zone_malloc(z, 64);
    check("a block of the new zone is found by malloc_zone_from_ptr", malloc_zone_from_ptr(p) == z, 0);
    free(p);

    malloc_zone_unregister(z);
    check("unregister: the count drops back", count() == n0, count());
    check("and it is not listed", !listed(z), 0);
    void *q = malloc_zone_malloc(z, 64);
    check("an unregistered zone still allocates", q != NULL && malloc_size(q) == 0, (long long)malloc_size(q));
    malloc_zone_free(z, q);
    malloc_zone_register(z);
    check("register: back in the list", count() == n0 + 1 && listed(z), count());
    malloc_destroy_zone(z);
    check("destroy: gone from the list", count() == n0 && !listed(z), count());
    return finish();
}
