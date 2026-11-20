/* poly-mac: corefoundation */
/* Test domain test.poly.cfspec.global. Besides the domain's own keys a read
 * also sees the user's global domain: a key every user has (AppleLanguages)
 * is found through any application id, and kCFPreferencesAnyApplication
 * names that global domain, which does not hold the test domain's keys. */
#include "poly_mcf.h"

int test_main(void) {
    CFStringRef app = CFSTR("test.poly.cfspec.global");
    cf_prefs_clean(app);
    CFTypeRef langs = CFPreferencesCopyAppValue(CFSTR("AppleLanguages"), app);
    CK(langs != NULL && CFGetTypeID(langs) == CFArrayGetTypeID() && CFArrayGetCount(langs) >= 1);
    CFTypeRef g = CFPreferencesCopyAppValue(CFSTR("AppleLanguages"), kCFPreferencesAnyApplication);
    CK(g != NULL && CFEqual(g, langs));
    CFPreferencesSetAppValue(CFSTR("unrelated"), CFSTR("own"), app);
    CK(CFEqual(CFPreferencesCopyAppValue(CFSTR("AppleLanguages"), app), langs));
    CK(CFPreferencesCopyAppValue(CFSTR("unrelated"), kCFPreferencesAnyApplication) == NULL);
    CK(CFPreferencesCopyValue(CFSTR("only-here"), app, kCFPreferencesCurrentUser, kCFPreferencesAnyHost) == NULL);
    cf_prefs_clean(app);
    return finish();
}
