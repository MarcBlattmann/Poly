/* poly-mac: corefoundation */
/* Test domain test.poly.cfspec.copyin. The preferences keep their own copy: a
 * mutable string, array or dictionary changed after being set does not change
 * what is read; the caller's reference count is not left raised for
 * immutable values that were handed in; a collection of property list types
 * is accepted whole. */
#include "poly_mcf.h"

int test_main(void) {
    CFStringRef app = CFSTR("test.poly.cfspec.copyin");
    cf_prefs_clean(app);
    CFMutableArrayRef a = CFArrayCreateMutable(kCFAllocatorDefault, 0, &kCFTypeArrayCallBacks);
    CFArrayAppendValue(a, CFSTR("one"));
    CFPreferencesSetAppValue(CFSTR("arr"), a, app);
    CFArrayAppendValue(a, CFSTR("two"));
    CFArrayRef got = CFPreferencesCopyAppValue(CFSTR("arr"), app);
    CK(got != NULL && CFArrayGetCount(got) == 1);
    CFMutableDictionaryRef d = CFDictionaryCreateMutable(kCFAllocatorDefault, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CFDictionarySetValue(d, CFSTR("a"), CFSTR("1"));
    CFPreferencesSetAppValue(CFSTR("dict"), d, app);
    CFDictionarySetValue(d, CFSTR("b"), CFSTR("2"));
    CFDictionaryRef gd = CFPreferencesCopyAppValue(CFSTR("dict"), app);
    CK(gd != NULL && CFDictionaryGetCount(gd) == 1);
    CFDataRef data = CFDataCreate(kCFAllocatorDefault, (const UInt8 *)"xyz", 3);
    CFIndex rc = CFGetRetainCount(data);
    CFPreferencesSetAppValue(CFSTR("data"), data, app);
    CFPreferencesSetAppValue(CFSTR("data"), NULL, app);
    CFPreferencesAppSynchronize(app);
    CKV(CFGetRetainCount(data) == rc, CFGetRetainCount(data) - rc);
    CFArrayRef nested = CFArrayCreate(kCFAllocatorDefault, (const void *[]){CFDictionaryCreate(kCFAllocatorDefault, (const void *[]){CFSTR("k")}, (const void *[]){kCFBooleanFalse}, 1, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks), CFSTR("s")}, 2, &kCFTypeArrayCallBacks);
    CFPreferencesSetAppValue(CFSTR("nested"), nested, app);
    CFArrayRef gn = CFPreferencesCopyAppValue(CFSTR("nested"), app);
    CK(gn != NULL && CFEqual(gn, nested));
    cf_prefs_clean(app);
    return finish();
}
