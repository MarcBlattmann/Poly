/* poly-mac: appkit */
/* The fixed colours: blackColor and whiteColor are in the generic gray space with 0 and 1 white and alpha 1;
 * clearColor has alpha 0; redColor is the sRGB colour (1,0,0,1) (and greenColor (0,1,0), blueColor (0,0,1));
 * blackColor equals colorWithWhite:0 alpha:1; the colours are shared instances (two calls return the same
 * object); colour objects are immutable and copying gives an equal one; a colour with alpha 0.5 reports
 * 0.5 and withAlphaComponent: returns a new colour with the changed alpha and the same colour components. */
#include "poly_mak.h"

int test_main(void) {
    NSColor *k = NSColor.blackColor, *w = NSColor.whiteColor;
    CKF(k.whiteComponent, 0);
    CKF(w.whiteComponent, 1);
    CKF(k.alphaComponent, 1);
    CKF(NSColor.clearColor.alphaComponent, 0);
    CK([k isEqual:[NSColor colorWithWhite:0 alpha:1]]);
    CK(k.colorSpace == [NSColorSpace genericGamma22GrayColorSpace]);
    NSColor *r = NSColor.redColor;
    CK(r.colorSpace == [NSColorSpace sRGBColorSpace]);
    CKF(r.redComponent, 1);
    CKF(r.greenComponent, 0);
    CKF(NSColor.greenColor.greenComponent, 1);
    CKF(NSColor.blueColor.blueComponent, 1);
    CK(NSColor.blackColor == NSColor.blackColor);
    CK([[r copy] isEqual:r]);
    NSColor *h = [r colorWithAlphaComponent:0.5];
    CKF(h.alphaComponent, 0.5);
    CKF(h.redComponent, 1);
    CKF(r.alphaComponent, 1);
    return finish();
}
