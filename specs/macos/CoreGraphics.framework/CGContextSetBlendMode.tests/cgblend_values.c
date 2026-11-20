/* The values of the blend mode constants, and the default mode. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    value("kCGBlendModeNormal", kCGBlendModeNormal);
    value("Multiply", kCGBlendModeMultiply);
    value("Screen", kCGBlendModeScreen);
    value("Overlay", kCGBlendModeOverlay);
    value("Darken", kCGBlendModeDarken);
    value("Lighten", kCGBlendModeLighten);
    value("ColorDodge", kCGBlendModeColorDodge);
    value("ColorBurn", kCGBlendModeColorBurn);
    value("SoftLight", kCGBlendModeSoftLight);
    value("HardLight", kCGBlendModeHardLight);
    value("Difference", kCGBlendModeDifference);
    value("Exclusion", kCGBlendModeExclusion);
    value("Hue", kCGBlendModeHue);
    value("Saturation", kCGBlendModeSaturation);
    value("Color", kCGBlendModeColor);
    value("Luminosity", kCGBlendModeLuminosity);
    value("Clear", kCGBlendModeClear);
    value("Copy", kCGBlendModeCopy);
    value("SourceIn", kCGBlendModeSourceIn);
    value("SourceOut", kCGBlendModeSourceOut);
    value("SourceAtop", kCGBlendModeSourceAtop);
    value("DestinationOver", kCGBlendModeDestinationOver);
    value("DestinationIn", kCGBlendModeDestinationIn);
    value("DestinationOut", kCGBlendModeDestinationOut);
    value("DestinationAtop", kCGBlendModeDestinationAtop);
    value("XOR", kCGBlendModeXOR);
    value("PlusDarker", kCGBlendModePlusDarker);
    value("PlusLighter", kCGBlendModePlusLighter);
    pxbuf b = px_rgba(1, 1);
    px_backdrop(&b, 0, 0, 1, 1);
    CGContextSetRGBFillColor(b.ctx, 1, 0, 0, 0.5);
    CGContextFillRect(b.ctx, CGRectMake(0, 0, 1, 1));
    px_value("the default mode is normal: 50% red over opaque blue", &b, 0, 0);
    return finish();
}
