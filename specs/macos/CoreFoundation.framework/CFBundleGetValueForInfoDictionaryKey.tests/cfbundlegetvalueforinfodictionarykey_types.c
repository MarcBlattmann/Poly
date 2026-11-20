/* poly-mac: corefoundation */
/* The value is the plist value as its CF type: a string, a number (integers
 * are 64-bit), a Boolean, an array; a key that is not there gives NULL; keys
 * are case sensitive; the values are not retained for the caller. */
#include "poly_mcf.h"

int test_main(void) {
    if (!cf_enter_app(CF_APP_PLIST)) return 1;
    CFBundleRef m = CFBundleGetMainBundle();
    CFTypeRef name = CFBundleGetValueForInfoDictionaryKey(m, CFSTR("CFBundleName"));
    CK(name != NULL && CFGetTypeID(name) == CFStringGetTypeID() && cf_is(name, "Probe"));
    CK(CFBundleGetValueForInfoDictionaryKey(m, kCFBundleNameKey) == name);
    CK(cf_is(CFBundleGetValueForInfoDictionaryKey(m, CFSTR("CFBundleVersion")), "42"));
    CK(cf_is(CFBundleGetValueForInfoDictionaryKey(m, kCFBundleVersionKey), "42"));
    CK(cf_is(CFBundleGetValueForInfoDictionaryKey(m, CFSTR("CFBundleShortVersionString")), "1.2.3"));
    CFTypeRef n = CFBundleGetValueForInfoDictionaryKey(m, CFSTR("MyInt"));
    long long out = 0;
    CK(n != NULL && CFGetTypeID(n) == CFNumberGetTypeID());
    CK(CFNumberGetValue(n, kCFNumberLongLongType, &out) && out == 7);
    CK(CFNumberGetType(n) == kCFNumberSInt64Type);
    CFTypeRef b = CFBundleGetValueForInfoDictionaryKey(m, CFSTR("MyBool"));
    CK(b == kCFBooleanTrue);
    CFTypeRef arr = CFBundleGetValueForInfoDictionaryKey(m, CFSTR("MyArr"));
    CK(arr != NULL && CFGetTypeID(arr) == CFArrayGetTypeID() && CFArrayGetCount(arr) == 2);
    CK(cf_is(CFArrayGetValueAtIndex(arr, 1), "b"));
    CK(CFBundleGetValueForInfoDictionaryKey(m, CFSTR("NoSuchKey")) == NULL);
    CK(CFBundleGetValueForInfoDictionaryKey(m, CFSTR("cfbundlename")) == NULL);
    CK(CFBundleGetValueForInfoDictionaryKey(m, CFSTR("")) == NULL);
    return finish();
}
