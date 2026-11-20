/* poly-mac: corefoundation */
/* Test domain test.poly.cfspec.copy, cleaned before and after. A key that was
 * never set gives NULL; each property list type comes back as the same type
 * with the same content; a returned value is the caller's own reference and
 * does not change when the original does. */
#include "poly_mcf.h"

int test_main(void) {
    CFStringRef app = CFSTR("test.poly.cfspec.copy");
    cf_prefs_clean(app);
    CK(CFPreferencesCopyAppValue(CFSTR("never-set"), app) == NULL);
    CFPreferencesSetAppValue(CFSTR("string"), CFSTR("text"), app);
    int n = 5;
    double d = 1.5;
    CFPreferencesSetAppValue(CFSTR("int"), CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &n), app);
    CFPreferencesSetAppValue(CFSTR("double"), CFNumberCreate(kCFAllocatorDefault, kCFNumberDoubleType, &d), app);
    CFPreferencesSetAppValue(CFSTR("bool"), kCFBooleanTrue, app);
    CFPreferencesSetAppValue(CFSTR("data"), CFDataCreate(kCFAllocatorDefault, (const UInt8 *)"ab", 2), app);
    CFPreferencesSetAppValue(CFSTR("array"), CFArrayCreate(kCFAllocatorDefault, (const void *[]){CFSTR("x"), CFSTR("y")}, 2, &kCFTypeArrayCallBacks), app);
    CFPreferencesSetAppValue(CFSTR("dict"), CFDictionaryCreate(kCFAllocatorDefault, (const void *[]){CFSTR("k")}, (const void *[]){CFSTR("v")}, 1, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks), app);
    CFPreferencesSetAppValue(CFSTR("date"), CFDateCreate(kCFAllocatorDefault, 1000.0), app);
    CK(cf_is(CFPreferencesCopyAppValue(CFSTR("string"), app), "text"));
    long long ll = 0;
    CFTypeRef vi = CFPreferencesCopyAppValue(CFSTR("int"), app);
    CK(vi && CFGetTypeID(vi) == CFNumberGetTypeID() && CFNumberGetValue(vi, kCFNumberLongLongType, &ll) && ll == 5);
    double dv = 0;
    CFTypeRef vd = CFPreferencesCopyAppValue(CFSTR("double"), app);
    CK(vd && CFNumberGetValue(vd, kCFNumberDoubleType, &dv) && dv == 1.5);
    CK(CFPreferencesCopyAppValue(CFSTR("bool"), app) == kCFBooleanTrue);
    CFDataRef data = CFPreferencesCopyAppValue(CFSTR("data"), app);
    CK(data && CFGetTypeID(data) == CFDataGetTypeID() && CFDataGetLength(data) == 2 && CFDataGetBytePtr(data)[1] == 'b');
    CFArrayRef arr = CFPreferencesCopyAppValue(CFSTR("array"), app);
    CK(arr && CFGetTypeID(arr) == CFArrayGetTypeID() && CFArrayGetCount(arr) == 2 && cf_is(CFArrayGetValueAtIndex(arr, 1), "y"));
    CFDictionaryRef dict = CFPreferencesCopyAppValue(CFSTR("dict"), app);
    CK(dict && CFGetTypeID(dict) == CFDictionaryGetTypeID() && cf_is(CFDictionaryGetValue(dict, CFSTR("k")), "v"));
    CFDateRef date = CFPreferencesCopyAppValue(CFSTR("date"), app);
    CK(date && CFGetTypeID(date) == CFDateGetTypeID() && CFDateGetAbsoluteTime(date) == 1000.0);
    CK(CFPreferencesCopyAppValue(CFSTR("STRING"), app) == NULL);
    CFMutableStringRef m = CFStringCreateMutable(kCFAllocatorDefault, 0);
    CFStringAppend(m, CFSTR("before"));
    CFPreferencesSetAppValue(CFSTR("mut"), m, app);
    CFStringAppend(m, CFSTR(" and after"));
    CK(cf_is(CFPreferencesCopyAppValue(CFSTR("mut"), app), "before"));
    cf_prefs_clean(app);
    CK(CFPreferencesCopyAppValue(CFSTR("string"), app) == NULL);
    return finish();
}
