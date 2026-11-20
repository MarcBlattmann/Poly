/* poly-mac: corefoundation */
/* The identifier is the CFBundleIdentifier string of the Info.plist; the
 * same string comes with kCFBundleIdentifierKey; CFBundleCreate of the .app's
 * URL gives the same bundle as the main one. */
#include "poly_mcf.h"

int test_main(void) {
    if (!cf_enter_app(CF_APP_PLIST)) return 1;
    CFBundleRef m = CFBundleGetMainBundle();
    CFStringRef id = CFBundleGetIdentifier(m);
    CK(id != NULL && CFGetTypeID(id) == CFStringGetTypeID());
    CK(cf_is(id, "test.poly.probe"));
    CK(CFBundleGetValueForInfoDictionaryKey(m, kCFBundleIdentifierKey) == id);
    CK(CFEqual(CFBundleGetValueForInfoDictionaryKey(m, CFSTR("CFBundleIdentifier")), id));
    CFURLRef u = CFBundleCopyBundleURL(m);
    CFBundleRef again = CFBundleCreate(kCFAllocatorDefault, u);
    CK(again == m);
    CK(CFBundleGetIdentifier(again) == id);
    return finish();
}
