/* poly-mac: appkit */
/* sizeToFit sets the frame size to the button's intrinsic size for its title and keeps the origin: after
 * title "OK" the width grows from the old size to the intrinsic width; a longer title gives a wider button;
 * an empty title gives the width of the bezel's two margins; the height does not depend on the title
 * (same for all). The numbers are of this macOS and its system font (SYSTEM-DEPENDENT, printed as sys_*):
 * "OK" 42.5 x 24, "Hello World" 94.5 x 24, "" 34 x 24 (regular size, automatic bezel). */
#include "poly_mak.h"

int test_main(void) {
    NSButton *b = [[NSButton alloc] initWithFrame:NSMakeRect(7, 9, 100, 30)];
    b.title = @"OK";
    [b sizeToFit];
    CKF(b.frame.origin.x, 7);
    CKF(b.frame.origin.y, 9);
    double w1 = b.frame.size.width, h1 = b.frame.size.height;
    CK(NSEqualSizes(b.frame.size, b.intrinsicContentSize));
    b.title = @"Hello World";
    [b sizeToFit];
    double w2 = b.frame.size.width;
    CK(w2 > w1 && b.frame.size.height == h1);
    b.title = @"";
    [b sizeToFit];
    double w0 = b.frame.size.width;
    CK(w0 < w1 && w0 > 0 && b.frame.size.height == h1);
    CKF(w2 - w1, 52);
    CKF(w1 - w0, 8.5);
    value_f("sys_ok_w", w1);
    value_f("sys_ok_h", h1);
    value_f("sys_hello_world_w", w2);
    value_f("sys_empty_w", w0);
    return finish();
}
