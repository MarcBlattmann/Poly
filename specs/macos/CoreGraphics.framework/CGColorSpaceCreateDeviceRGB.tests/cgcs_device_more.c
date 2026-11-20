/* Device gray and device CMYK counterparts: components and models. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    CGColorSpaceRef g = CGColorSpaceCreateDeviceGray();
    CGColorSpaceRef k = CGColorSpaceCreateDeviceCMYK();
    check("device gray: 1 component, model monochrome (0)", CGColorSpaceGetNumberOfComponents(g) == 1 && CGColorSpaceGetModel(g) == kCGColorSpaceModelMonochrome, 0);
    check("device CMYK: 4 components, model CMYK (2)", CGColorSpaceGetNumberOfComponents(k) == 4 && CGColorSpaceGetModel(k) == kCGColorSpaceModelCMYK, 0);
    check("the three device spaces are different objects", g != k && g != CGColorSpaceCreateDeviceRGB(), 0);
    value("model values: monochrome", kCGColorSpaceModelMonochrome);
    value("RGB", kCGColorSpaceModelRGB);
    value("CMYK", kCGColorSpaceModelCMYK);
    value("lab", kCGColorSpaceModelLab);
    value("indexed", kCGColorSpaceModelIndexed);
    value("pattern", kCGColorSpaceModelPattern);
    check("retain/release keep the object alive", ({ CGColorSpaceRef r = CGColorSpaceRetain(g); CGColorSpaceRelease(r); CGColorSpaceGetNumberOfComponents(g) == 1; }), 0);
    return finish();
}
