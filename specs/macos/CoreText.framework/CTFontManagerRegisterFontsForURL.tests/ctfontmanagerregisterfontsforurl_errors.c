/* poly-mac: coretext */
/* Registration errors, all in the domain com.apple.CoreText.CTFontManagerErrorDomain:
 * a missing file (code 101), a file that is not a font (103), a font registered twice
 * (105, the first registration stays). Each returns false and sets the error when an
 * error pointer is given; a NULL error pointer is accepted. */
#include "poly_mct.h"

static CFURLRef url(const char *name) {
    CFStringRef s = CFStringCreateWithCString(kCFAllocatorDefault, name, kCFStringEncodingUTF8);
    return CFURLCopyAbsoluteURL(CFURLCreateWithFileSystemPath(kCFAllocatorDefault, s, kCFURLPOSIXPathStyle, false));
}

int test_main(void) {
    const char *domain = "com.apple.CoreText.CTFontManagerErrorDomain";
    CFErrorRef err = NULL;
    CK(!CTFontManagerRegisterFontsForURL(url("nothere.ttf"), kCTFontManagerScopeProcess, &err));
    CK(err != NULL && CFErrorGetCode(err) == 101 && cf_is(CFErrorGetDomain(err), domain));
    CK(cf_write_file("junk.ttf", "this is not a font file at all", 29));
    err = NULL;
    CK(!CTFontManagerRegisterFontsForURL(url("junk.ttf"), kCTFontManagerScopeProcess, &err));
    CK(err != NULL && CFErrorGetCode(err) == 103 && cf_is(CFErrorGetDomain(err), domain));
    CK(cf_write_file("PolyTest-Regular.ttf", (const char *)poly_font_regular, sizeof poly_font_regular));
    err = NULL;
    CK(CTFontManagerRegisterFontsForURL(url("PolyTest-Regular.ttf"), kCTFontManagerScopeProcess, &err) && err == NULL);
    err = NULL;
    CK(!CTFontManagerRegisterFontsForURL(url("PolyTest-Regular.ttf"), kCTFontManagerScopeProcess, &err));
    CK(err != NULL && CFErrorGetCode(err) == 105 && cf_is(CFErrorGetDomain(err), domain));
    CK(pt_is(CTFontCreateWithName(CFSTR("PolyTest-Regular"), 20, NULL), "PolyTest-Regular"));
    CK(!CTFontManagerRegisterFontsForURL(url("nothere.ttf"), kCTFontManagerScopeProcess, NULL));
    return finish();
}
