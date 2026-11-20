/* poly-mac: coretext */
/* A font file registered for the process (scope process) is found by CTFontCreateWithName
 * under its PostScript name, its full name and its family name, with any letter case;
 * before the registration none of these names finds it; registering makes the family
 * appear in the available family names and the call reports true with no error. */
#include "poly_mct.h"

static int has_family(const char *name) {
    CFArrayRef a = CTFontManagerCopyAvailableFontFamilyNames();
    CFStringRef s = CFStringCreateWithCString(kCFAllocatorDefault, name, kCFStringEncodingUTF8);
    return a && CFArrayContainsValue(a, CFRangeMake(0, CFArrayGetCount(a)), s);
}

int test_main(void) {
    CK(!pt_is(CTFontCreateWithName(CFSTR("PolyTest-Regular"), 20, NULL), "PolyTest-Regular"));
    CK(!pt_is(CTFontCreateWithName(CFSTR("PolyTest"), 20, NULL), "PolyTest-Regular"));
    CK(!has_family("PolyTest"));
    CK(cf_write_file("PolyTest-Regular.ttf", (const char *)poly_font_regular, sizeof poly_font_regular));
    CFURLRef url = CFURLCopyAbsoluteURL(CFURLCreateWithFileSystemPath(kCFAllocatorDefault, CFSTR("PolyTest-Regular.ttf"), kCFURLPOSIXPathStyle, false));
    CFErrorRef err = NULL;
    CK(CTFontManagerRegisterFontsForURL(url, kCTFontManagerScopeProcess, &err));
    CK(err == NULL);
    CK(pt_is(CTFontCreateWithName(CFSTR("PolyTest-Regular"), 20, NULL), "PolyTest-Regular"));
    CK(pt_is(CTFontCreateWithName(CFSTR("PolyTest Regular"), 20, NULL), "PolyTest-Regular"));
    CK(pt_is(CTFontCreateWithName(CFSTR("PolyTest"), 20, NULL), "PolyTest-Regular"));
    CK(pt_is(CTFontCreateWithName(CFSTR("polytest-regular"), 20, NULL), "PolyTest-Regular"));
    CK(pt_is(CTFontCreateWithName(CFSTR("POLYTEST"), 20, NULL), "PolyTest-Regular"));
    CK(has_family("PolyTest"));
    CK(!pt_is(CTFontCreateWithName(CFSTR("PolyTest-Bold"), 20, NULL), "PolyTest-Bold"));
    return finish();
}
