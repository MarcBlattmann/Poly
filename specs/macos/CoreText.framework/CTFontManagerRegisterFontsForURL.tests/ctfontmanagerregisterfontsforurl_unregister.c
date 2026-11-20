/* poly-mac: coretext */
/* A registered font can be unregistered (true, no error); lookups by name then no longer find
 * it and its family disappears from the family names; a second unregistration
 * fails with code 201 (not registered); it can be registered again and is found
 * again; two fonts of one family (Regular, Bold) register separately and
 * the Bold one is found by CTFontCreateCopyWithSymbolicTraits from the Regular. */
#include "poly_mct.h"

static CFURLRef url(const char *name) {
    CFStringRef s = CFStringCreateWithCString(kCFAllocatorDefault, name, kCFStringEncodingUTF8);
    return CFURLCopyAbsoluteURL(CFURLCreateWithFileSystemPath(kCFAllocatorDefault, s, kCFURLPOSIXPathStyle, false));
}

int test_main(void) {
    CK(pt_register());
    CK(pt_is(CTFontCreateWithName(CFSTR("PolyTest-Bold"), 20, NULL), "PolyTest-Bold"));
    CTFontRef reg = CTFontCreateWithName(CFSTR("PolyTest-Regular"), 20, NULL);
    CK(pt_is(CTFontCreateCopyWithSymbolicTraits(reg, 0, NULL, kCTFontTraitBold, kCTFontTraitBold), "PolyTest-Bold"));
    CFErrorRef err = NULL;
    CK(CTFontManagerUnregisterFontsForURL(url("PolyTest-Regular.ttf"), kCTFontManagerScopeProcess, &err) && err == NULL);
    CK(!pt_is(CTFontCreateWithName(CFSTR("PolyTest-Regular"), 20, NULL), "PolyTest-Regular"));
    CK(pt_is(CTFontCreateWithName(CFSTR("PolyTest-Bold"), 20, NULL), "PolyTest-Bold"));
    err = NULL;
    CK(!CTFontManagerUnregisterFontsForURL(url("PolyTest-Regular.ttf"), kCTFontManagerScopeProcess, &err));
    CK(err != NULL && CFErrorGetCode(err) == 201);
    CK(CTFontManagerRegisterFontsForURL(url("PolyTest-Regular.ttf"), kCTFontManagerScopeProcess, NULL));
    CK(pt_is(CTFontCreateWithName(CFSTR("PolyTest-Regular"), 20, NULL), "PolyTest-Regular"));
    CK(CTFontManagerUnregisterFontsForURL(url("PolyTest-Bold.ttf"), kCTFontManagerScopeProcess, NULL));
    CK(!pt_is(CTFontCreateWithName(CFSTR("PolyTest-Bold"), 20, NULL), "PolyTest-Bold"));
    return finish();
}
