/* CGColorSpaceCreateWithName: the named spaces Chrome asks for; each has a
 * model, a number of components and gives back the name. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static void show(const char *label, CFStringRef n) {
    CGColorSpaceRef cs = CGColorSpaceCreateWithName(n);
    if (!cs) {
        value_str(label, "NULL");
        return;
    }
    char buf[128] = "(no name)";
    CFStringRef got = CGColorSpaceCopyName(cs);
    if (got) {
        CFStringGetCString(got, buf, sizeof buf, kCFStringEncodingUTF8);
        CFRelease(got);
    }
    char line[256];
    size_t k = 0;
    for (const char *p = "model "; *p; p++) line[k++] = *p;
    line[k++] = (char)('0' + (int)CGColorSpaceGetModel(cs));
    for (const char *p = " components "; *p; p++) line[k++] = *p;
    line[k++] = (char)('0' + (int)CGColorSpaceGetNumberOfComponents(cs));
    for (const char *p = " name "; *p; p++) line[k++] = *p;
    for (const char *p = buf; *p && k < 250; p++) line[k++] = *p;
    line[k] = 0;
    value_str(label, line);
}

int test_main(void) {
    show("sRGB", kCGColorSpaceSRGB);
    show("DisplayP3", kCGColorSpaceDisplayP3);
    show("GenericRGB", kCGColorSpaceGenericRGB);
    show("GenericGray", kCGColorSpaceGenericGray);
    show("GenericCMYK", kCGColorSpaceGenericCMYK);
    show("ExtendedSRGB", kCGColorSpaceExtendedSRGB);
    show("LinearSRGB", kCGColorSpaceLinearSRGB);
    show("ExtendedLinearSRGB", kCGColorSpaceExtendedLinearSRGB);
    show("ExtendedLinearITUR_2020", kCGColorSpaceExtendedLinearITUR_2020);
    show("ITUR_2100_PQ", kCGColorSpaceITUR_2100_PQ);
    show("ITUR_2100_HLG", kCGColorSpaceITUR_2100_HLG);
    show("GenericRGBLinear", kCGColorSpaceGenericRGBLinear);
    show("AdobeRGB1998", kCGColorSpaceAdobeRGB1998);
    show("an unknown name", CFSTR("poly-no-such-space"));
    check("sRGB is the same object on every call", CGColorSpaceCreateWithName(kCGColorSpaceSRGB) == CGColorSpaceCreateWithName(kCGColorSpaceSRGB), 0);
    check("an unknown name gives NULL", CGColorSpaceCreateWithName(CFSTR("poly-no-such-space")) == NULL, 0);
    check("the name of sRGB is kCGColorSpaceSRGB", ({ CFStringRef n = CGColorSpaceCopyName(CGColorSpaceCreateWithName(kCGColorSpaceSRGB)); int ok = n && CFEqual(n, kCGColorSpaceSRGB); if (n) CFRelease(n); ok; }), 0);
    return finish();
}
