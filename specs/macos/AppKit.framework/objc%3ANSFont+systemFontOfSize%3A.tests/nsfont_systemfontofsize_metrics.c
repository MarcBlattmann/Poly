/* poly-mac: appkit */
/* The metrics of the 13 point system font (SYSTEM-DEPENDENT, printed as sys_*; font units scale with the size):
 * ascender 12.57, descender -2.74, leading 0, capHeight 9.16, xHeight 6.84, italicAngle 0, bounding box
 * (-8.39,-3.58,40.45,16.02), the advance of "H" 9.57, "Hello" is 30.95 wide and 16 high (sizeWithAttributes:), and
 * boundingRectWithSize for "Hello" is (0,-3,30.95,16). What is fixed: ascender > capHeight > xHeight > 0 >
 * descender, the metrics grow in proportion to the size (a 26 point font has twice the 13 point ones within
 * 0.05), and the width of a text is the sum of the advances (the line "HH" is twice as wide as "H"
 * within 0.05, without kerning between equal letters). */
#include "poly_mak.h"

static double absd(double v) { return v < 0 ? -v : v; }

int test_main(void) {
    NSFont *f = [NSFont systemFontOfSize:13], *g = [NSFont systemFontOfSize:26];
    CK(f.ascender > f.capHeight && f.capHeight > f.xHeight && f.xHeight > 0 && f.descender < 0);
    CK(near(g.ascender, 2 * f.ascender) || absd(g.ascender - 2 * f.ascender) < 0.05);
    CK(absd(g.capHeight - 2 * f.capHeight) < 0.05);
    NSDictionary *at = @{NSFontAttributeName: f};
    double h = [@"H" sizeWithAttributes:at].width, hh = [@"HH" sizeWithAttributes:at].width;
    CK(absd(hh - 2 * h) < 0.05);
    value_f("sys_ascender", f.ascender);
    value_f("sys_descender", f.descender);
    value_f("sys_leading", f.leading);
    value_f("sys_capHeight", f.capHeight);
    value_f("sys_xHeight", f.xHeight);
    value_f("sys_italicAngle", f.italicAngle);
    NSRect bb = f.boundingRectForFont;
    value_f("sys_bbox_x", bb.origin.x);
    value_f("sys_bbox_y", bb.origin.y);
    value_f("sys_bbox_w", bb.size.width);
    value_f("sys_bbox_h", bb.size.height);
    value_f("sys_H_advance", [f advancementForGlyph:[f glyphWithName:@"H"]].width);
    NSSize hello = [@"Hello" sizeWithAttributes:at];
    value_f("sys_hello_w", hello.width);
    value_f("sys_hello_h", hello.height);
    NSRect br = [@"Hello" boundingRectWithSize:NSMakeSize(1000, 1000) options:0 attributes:at];
    value_f("sys_hello_bound_y", br.origin.y);
    value_f("sys_hello_bound_h", br.size.height);
    return finish();
}
