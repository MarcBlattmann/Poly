/* poly-mac: iosurface
 * Properties that fix what the surface would otherwise choose: BytesPerRow is kept as given when it is at least width * bytes per element (400, 401,
 * 1000, 5000 for 100 elements of 4 bytes: not aligned), and the allocation grows with it; AllocSize is kept exactly (1000000: not rounded to pages);
 * an element size of 2 by 2 halves the elements across and down; a name set with kIOSurfaceName and the cache mode are accepted. */
#include "poly_miosurface.h"
int test_main(void) {
    long long rows[] = {400, 401, 1000, 5000};
    for (int i = 0; i < 4; i++) {
        CFMutableDictionaryRef d = ios_props(100, 50, 4, 'BGRA');
        ios_set_int(d, kIOSurfaceBytesPerRow, rows[i]);
        IOSurfaceRef s = ios_make_props(d);
        char name[64];
        snprintf(name, sizeof name, "BytesPerRow %lld is kept", rows[i]);
        check(name, s != NULL && IOSurfaceGetBytesPerRow(s) == (size_t)rows[i], s ? (long long)IOSurfaceGetBytesPerRow(s) : -1);
        if (s) check("and the allocation holds all the rows", IOSurfaceGetAllocSize(s) >= (size_t)rows[i] * 49 + 400, IOSurfaceGetAllocSize(s));
    }
    CFMutableDictionaryRef d = ios_props(100, 50, 4, 'BGRA');
    ios_set_int(d, kIOSurfaceAllocSize, 1000000);
    IOSurfaceRef a = ios_make_props(d);
    check("AllocSize 1000000 is kept exactly", a != NULL && IOSurfaceGetAllocSize(a) == 1000000, a ? (long long)IOSurfaceGetAllocSize(a) : -1);
    d = ios_props(100, 50, 4, 'BGRA');
    ios_set_int(d, kIOSurfaceElementWidth, 2);
    ios_set_int(d, kIOSurfaceElementHeight, 2);
    IOSurfaceRef e = ios_make_props(d);
    check("an element of 2 by 2 pixels", e != NULL && IOSurfaceGetElementWidth(e) == 2 && IOSurfaceGetElementHeight(e) == 2, 0);
    check("its rows are of 50 elements of 4 bytes, aligned", e && IOSurfaceGetBytesPerRow(e) == 256, e ? (long long)IOSurfaceGetBytesPerRow(e) : -1);
    d = ios_props(100, 50, 4, 'BGRA');
    CFStringRef n = ios_cfstr("poly-surface-name");
    CFDictionarySetValue(d, kIOSurfaceName, n);
    IOSurfaceRef named = ios_make_props(d);
    CFTypeRef back = named ? IOSurfaceCopyValue(named, kIOSurfaceName) : NULL;
    check("a name is accepted and read back with IOSurfaceCopyValue", back && CFEqual(back, n), 0);
    d = ios_props(100, 50, 4, 'BGRA');
    ios_set_int(d, kIOSurfaceCacheMode, 0);
    check("cache mode 0 is accepted", ios_make_props(d) != NULL, 0);
    return finish();
}
