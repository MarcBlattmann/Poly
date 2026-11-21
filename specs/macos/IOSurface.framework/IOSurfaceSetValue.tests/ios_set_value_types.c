/* poly-mac: iosurface
 * IOSurfaceSetValue(surface, key, value) attaches a CF property list value to the surface and IOSurfaceCopyValue gives it back: strings, numbers, data, dictionaries,
 * arrays and booleans keep their type and contents; setting the same key again replaces the value; the copy is a new reference the caller releases. */
#include "poly_miosurface.h"
int test_main(void) {
    IOSurfaceRef s = ios_make(32, 32, 4);
    IOSurfaceSetValue(s, CFSTR("poly.string"), CFSTR("hello"));
    CFTypeRef v = IOSurfaceCopyValue(s, CFSTR("poly.string"));
    check("a string comes back equal", ios_str_eq(v, "hello"), 0);
    int n = 42;
    CFNumberRef num = CFNumberCreate(NULL, kCFNumberIntType, &n);
    IOSurfaceSetValue(s, CFSTR("poly.number"), num);
    v = IOSurfaceCopyValue(s, CFSTR("poly.number"));
    check("a number comes back as a number with the same value", v && CFGetTypeID(v) == CFNumberGetTypeID() && ios_num(v) == 42, 0);
    unsigned char bytes[4] = {1, 2, 3, 4};
    CFDataRef data = CFDataCreate(NULL, bytes, 4);
    IOSurfaceSetValue(s, CFSTR("poly.data"), data);
    v = IOSurfaceCopyValue(s, CFSTR("poly.data"));
    check("data comes back with its bytes", v && CFGetTypeID(v) == CFDataGetTypeID() && CFDataGetLength(v) == 4 && CFDataGetBytePtr(v)[3] == 4, 0);
    CFMutableDictionaryRef dict = CFDictionaryCreateMutable(NULL, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CFDictionarySetValue(dict, CFSTR("x"), CFSTR("y"));
    IOSurfaceSetValue(s, CFSTR("poly.dict"), dict);
    v = IOSurfaceCopyValue(s, CFSTR("poly.dict"));
    check("a dictionary comes back with its entry", v && CFGetTypeID(v) == CFDictionaryGetTypeID() && ios_str_eq(CFDictionaryGetValue(v, CFSTR("x")), "y"), 0);
    CFStringRef items[2] = {CFSTR("a"), CFSTR("b")};
    CFArrayRef arr = CFArrayCreate(NULL, (const void **)items, 2, &kCFTypeArrayCallBacks);
    IOSurfaceSetValue(s, CFSTR("poly.array"), arr);
    v = IOSurfaceCopyValue(s, CFSTR("poly.array"));
    check("an array comes back with its items", v && CFGetTypeID(v) == CFArrayGetTypeID() && CFArrayGetCount(v) == 2 && ios_str_eq(CFArrayGetValueAtIndex(v, 1), "b"), 0);
    IOSurfaceSetValue(s, CFSTR("poly.bool"), kCFBooleanTrue);
    v = IOSurfaceCopyValue(s, CFSTR("poly.bool"));
    check("a boolean comes back true", v == kCFBooleanTrue, 0);
    IOSurfaceSetValue(s, CFSTR("poly.string"), CFSTR("changed"));
    v = IOSurfaceCopyValue(s, CFSTR("poly.string"));
    check("setting a key again replaces the value", ios_str_eq(v, "changed"), 0);
    return finish();
}
