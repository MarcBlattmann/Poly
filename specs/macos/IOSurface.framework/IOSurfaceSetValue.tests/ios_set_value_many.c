/* poly-mac: iosurface
 * Many attachments: a hundred keys with different values all read back; a long string value (100000 characters) and a data value of one megabyte are kept in
 * full; a key of 300 characters works. IOSurfaceCopyAllValues has the attachments among what it returns. */
#include "poly_miosurface.h"
int test_main(void) {
    IOSurfaceRef s = ios_make(64, 64, 4);
    char key[16];
    for (int i = 0; i < 100; i++) {
        snprintf(key, sizeof key, "poly.k%d", i);
        CFStringRef k = ios_cfstr(key);
        int n = i * 7;
        CFNumberRef v = CFNumberCreate(NULL, kCFNumberIntType, &n);
        IOSurfaceSetValue(s, k, v);
    }
    int ok = 1;
    for (int i = 0; i < 100; i++) {
        snprintf(key, sizeof key, "poly.k%d", i);
        ok &= ios_num(IOSurfaceCopyValue(s, ios_cfstr(key))) == i * 7;
    }
    check("a hundred keys read back", ok, 0);
    char *big = malloc(100001);
    memset(big, 'x', 100000);
    big[100000] = 0;
    IOSurfaceSetValue(s, CFSTR("poly.long"), ios_cfstr(big));
    check("a string of 100000 characters is kept", ios_str_len(IOSurfaceCopyValue(s, CFSTR("poly.long"))) == 100000, 0);
    unsigned char *mb = calloc(1, 1 << 20);
    CFDataRef data = CFDataCreate(NULL, mb, 1 << 20);
    IOSurfaceSetValue(s, CFSTR("poly.mb"), data);
    CFTypeRef back = IOSurfaceCopyValue(s, CFSTR("poly.mb"));
    check("one megabyte of data is kept", back && CFDataGetLength(back) == (1 << 20), 0);
    char longkey[301];
    memset(longkey, 'k', 300);
    longkey[300] = 0;
    IOSurfaceSetValue(s, ios_cfstr(longkey), CFSTR("long key"));
    check("a key of 300 characters works", ios_str_eq(IOSurfaceCopyValue(s, ios_cfstr(longkey)), "long key"), 0);
    CFDictionaryRef all = IOSurfaceCopyAllValues(s);
    check("IOSurfaceCopyAllValues has the attachments", all && CFDictionaryContainsKey(all, CFSTR("poly.k50")) && CFDictionaryContainsKey(all, CFSTR("poly.long")), 0);
    return finish();
}
