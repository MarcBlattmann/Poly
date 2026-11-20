/* poly-mac: corefoundation */
/* A program that is not in a bundle still has a main bundle: it has no
 * identifier and an empty Info dictionary, its bundle URL is the directory of
 * the executable, and no resource is found in it. */
#include "poly_mcf.h"

int test_main(void) {
    CFBundleRef m = CFBundleGetMainBundle();
    CK(m != NULL);
    CK(CFBundleGetMainBundle() == m);
    CK(CFGetTypeID(m) == CFBundleGetTypeID());
    CK(CFBundleGetIdentifier(m) == NULL);
    CK(CFBundleGetValueForInfoDictionaryKey(m, CFSTR("CFBundleName")) == NULL);
    CFDictionaryRef info = CFBundleGetInfoDictionary(m);
    CK(info != NULL && CFDictionaryGetCount(info) == 0);
    CFURLRef url = CFBundleCopyBundleURL(m);
    CK(url != NULL);
    CK(CFURLHasDirectoryPath(url));
    CFURLRef exe = CFBundleCopyExecutableURL(m);
    CK(exe != NULL && !CFURLHasDirectoryPath(exe));
    CK(CFEqual(CFURLCreateCopyDeletingLastPathComponent(kCFAllocatorDefault, exe), url));
    CK(CFBundleCopyResourceURL(m, CFSTR("hello"), CFSTR("txt"), NULL) == NULL);
    UInt32 type = 0, creator = 0;
    CFBundleGetPackageInfo(m, &type, &creator);
    CKV(type == 0x424e444c, type);
    CK(CFBundleGetVersionNumber(m) == 0);
    return finish();
}
