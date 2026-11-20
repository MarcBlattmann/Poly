/* poly-mac: iosurface
 * IOSurfaceGetAllocSize of a surface made with no AllocSize: BytesPerRow * height rounded up to a multiple of 16384 bytes (the kernel's page size here):
 * 100 by 10 elements of 4 bytes is 16384, 100 by 100 is 65536, 100 by 1000 is 524288, 100 by 4096 is 2097152, 1000 by 10 is 49152, 4000 by 10 is 163840. The
 * allocation always holds all the rows. */
#include "poly_miosurface.h"
int test_main(void) {
    struct {
        int w, h;
        size_t alloc;
    } t[] = {{1, 10, 16384}, {100, 1, 16384}, {100, 10, 16384}, {100, 100, 65536}, {100, 1000, 524288}, {100, 4096, 2097152}, {1000, 10, 49152}, {1920, 10, 81920}, {4000, 10, 163840}};
    for (unsigned i = 0; i < sizeof t / sizeof t[0]; i++) {
        IOSurfaceRef s = ios_make(t[i].w, t[i].h, 4);
        char name[64];
        snprintf(name, sizeof name, "%d by %d elements of 4 bytes: %zu", t[i].w, t[i].h, t[i].alloc);
        check(name, s && IOSurfaceGetAllocSize(s) == t[i].alloc, s ? (long long)IOSurfaceGetAllocSize(s) : -1);
    }
    IOSurfaceRef s = ios_make(777, 333, 4);
    check("the allocation holds all the rows", IOSurfaceGetAllocSize(s) >= IOSurfaceGetBytesPerRow(s) * 333, IOSurfaceGetAllocSize(s));
    check("and is a multiple of 16384", IOSurfaceGetAllocSize(s) % 16384 == 0, IOSurfaceGetAllocSize(s));
    return finish();
}
