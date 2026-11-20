/* poly-mac: corefoundation */
/* A resource that is only in a language directory (en.lproj) is found with
 * no subdirectory; the file behind the URL can be opened and holds the
 * bytes written; CFBundleCopyResourcesDirectoryURL names the directory the
 * lookups use. */
#include "poly_mcf.h"

int test_main(void) {
    if (!cf_enter_app(CF_APP_PLIST)) return 1;
    CFBundleRef m = CFBundleGetMainBundle();
    CFURLRef loc = CFBundleCopyResourceURL(m, CFSTR("loc"), CFSTR("txt"), NULL);
    CK(cf_url_path_ends(loc, "/Contents/Resources/en.lproj/loc.txt"));
    CFURLRef hello = CFBundleCopyResourceURL(m, CFSTR("hello"), CFSTR("txt"), NULL);
    char path[512];
    CFURLGetFileSystemRepresentation(hello, true, (UInt8 *)path, sizeof path);
    char buf[16];
    memset(buf, 0, sizeof buf);
    sysret fd = bsd3(SYS_open, path, 0, 0);
    CK(fd.err == 0);
    sysret r = bsd3(SYS_read, fd.ret, buf, sizeof buf);
    CK(r.ret == 3 && mem_eq(buf, "hi\n", 3));
    CFURLRef dirurl = CFBundleCopyResourcesDirectoryURL(m);
    CFURLRef abs = CFURLCopyAbsoluteURL(dirurl);
    CK(cf_url_path_ends(abs, "/T.app/Contents/Resources"));
    CK(CFBundleCopyResourceURLInDirectory(CFBundleCopyBundleURL(m), CFSTR("hello"), CFSTR("txt"), NULL) != NULL);
    CFArrayRef all = CFBundleCopyResourceURLsOfType(m, CFSTR("txt"), NULL);
    CK(all != NULL && CFArrayGetCount(all) >= 2);
    return finish();
}
