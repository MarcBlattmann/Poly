/* poly-mac: appkit */
/* +textFieldWithString: makes an editable field with that text: editable, selectable, bezeled, draws its
 * background, accepts first responder, line break mode clipping (2), and its frame starts at (0,0) sized to
 * fit the text ("Edit": 35.5 x 24, SYSTEM-DEPENDENT, printed); +textFieldWithString: of the empty string is
 * as wide as the field's margins (SYSTEM-DEPENDENT, printed); the field made with initWithFrame: has the same
 * flags but the line break mode word wrapping; the bezel style, border and background of both are the same;
 * a text field is a subclass of NSControl and NSView. */
#include "poly_mak.h"

int test_main(void) {
    NSTextField *e = [NSTextField textFieldWithString:@"Edit"];
    CK([e.stringValue isEqualToString:@"Edit"]);
    CK(e.isEditable && e.isSelectable && e.isBezeled && !e.isBordered && e.drawsBackground && e.acceptsFirstResponder);
    CK(e.lineBreakMode == NSLineBreakByClipping);
    CKF(e.frame.origin.x, 0);
    CKF(e.frame.origin.y, 0);
    value_f("sys_edit_w", e.frame.size.width);
    value_f("sys_edit_h", e.frame.size.height);
    NSTextField *z = [NSTextField textFieldWithString:@""];
    value_f("sys_empty_w", z.frame.size.width);
    NSTextField *f = [[NSTextField alloc] initWithFrame:NSMakeRect(0, 0, 100, 22)];
    CK(f.bezelStyle == e.bezelStyle && f.isBordered == e.isBordered && f.drawsBackground == e.drawsBackground);
    CK(f.lineBreakMode == NSLineBreakByWordWrapping);
    CK([e isKindOfClass:[NSControl class]] && [e isKindOfClass:[NSView class]]);
    return finish();
}
