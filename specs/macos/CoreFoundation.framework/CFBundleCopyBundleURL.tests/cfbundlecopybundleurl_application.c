/* poly-mac: corefoundation */
/* The bundle URL is a file URL naming the .app directory, with a directory
 * path (trailing slash); its file system path is the .app and the
 * URL is a new reference each call. */
#include "poly_mcf.h"

int test_main(void) {
    if (!cf_enter_app(CF_APP_PLIST)) return 1;
    CFBundleRef m = CFBundleGetMainBundle();
    CFURLRef u = CFBundleCopyBundleURL(m);
    CK(u != NULL && CFGetTypeID(u) == CFURLGetTypeID());
    CK(CFURLHasDirectoryPath(u));
    CFStringRef scheme = CFURLCopyScheme(u);
    CK(cf_is(scheme, "file"));
    CK(cf_url_path_ends(u, "/T.app"));
    CFStringRef s = CFURLGetString(u);
    CK(cf_ends(s, "/T.app/"));
    CK(CFURLCopyAbsoluteURL(u) != NULL);
    CFURLRef exe = CFBundleCopyExecutableURL(m);
    CK(cf_url_path_ends(exe, "/Contents/MacOS/t"));
    CFURLRef res = CFBundleCopyResourcesDirectoryURL(m);
    CK(cf_url_path_ends(res, "/T.app/Contents/Resources") || cf_url_path_ends(CFURLCopyAbsoluteURL(res), "/T.app/Contents/Resources"));
    CK(CFURLHasDirectoryPath(res));
    CFURLRef infoURL = CFBundleCopyBundleURL(CFBundleGetMainBundle());
    CK(CFEqual(u, infoURL));
    return finish();
}
