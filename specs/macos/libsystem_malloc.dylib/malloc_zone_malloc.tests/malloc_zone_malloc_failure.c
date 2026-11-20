/* malloc_zone_malloc and friends on impossible sizes: NULL with ENOMEM
 * (12); calloc overflow also; memalign with a bad alignment. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <errno.h>
#include <malloc/malloc.h>
#include <stdlib.h>

int test_main(void) {
    malloc_zone_t *z = malloc_default_zone();
    errno = 0;
    void *p = malloc_zone_malloc(z, (size_t)-1);
    check("zone_malloc(SIZE_MAX) is NULL", p == NULL, 0);
    value("errno after zone_malloc(SIZE_MAX) (the zone call leaves errno alone)", errno);
    p = malloc_zone_calloc(z, (size_t)-1, 2);
    check("zone_calloc overflow is NULL", p == NULL, 0);
    errno = 0;
    p = malloc((size_t)1 << 62);
    check("malloc(1<<62) is NULL with ENOMEM", p == NULL && errno == ENOMEM, errno);
    errno = 0;
    p = calloc((size_t)1 << 32, (size_t)1 << 32);
    check("calloc(2^32, 2^32) is NULL with ENOMEM", p == NULL && errno == ENOMEM, errno);
    void *a = malloc(10);
    errno = 0;
    void *b = realloc(a, (size_t)-1);
    check("realloc to SIZE_MAX fails with ENOMEM", b == NULL && errno == ENOMEM, errno);
    check("and leaves the original block valid", malloc_size(a) >= 10, (long long)malloc_size(a));
    free(a);
    return finish();
}
