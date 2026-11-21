/* poly-mac: appkit */
/* A text field from initWithFrame: keeps its frame exactly, has the empty string (not nil), is editable,
 * selectable, bezeled (bezel style square, 0) and not bordered, draws its background, is enabled, has
 * tag 0, the system font 13 points, the catalog colours `controlTextColor` for the text and
 * `textBackgroundColor` for the background, the line break mode NSLineBreakByWordWrapping (0), no single-line mode,
 * no line limit (maximumNumberOfLines 0), left alignment (NSTextAlignmentLeft, 0; the label and edit-field
 * factories use natural, 4), accepts first responder, is flipped, and has an intrinsic size
 * of -1 (no intrinsic metric, NSViewNoIntrinsicMetric) wide and 24 high (SYSTEM-DEPENDENT, printed). */
#include "poly_mak.h"

int test_main(void) {
    NSTextField *f = [[NSTextField alloc] initWithFrame:NSMakeRect(1, 2, 100, 22)];
    CK(NSEqualRects(f.frame, NSMakeRect(1, 2, 100, 22)));
    CK(f.stringValue != nil && f.stringValue.length == 0);
    CK(f.isEditable && f.isSelectable && f.isBezeled && !f.isBordered && f.drawsBackground && f.isEnabled);
    CK(f.bezelStyle == NSTextFieldSquareBezel && NSTextFieldSquareBezel == 0);
    CK(f.tag == 0);
    CK([f.font.fontName isEqualToString:@".AppleSystemUIFont"]);
    CKF(f.font.pointSize, 13);
    CK([f.textColor isEqual:NSColor.controlTextColor]);
    CK([f.backgroundColor isEqual:NSColor.textBackgroundColor]);
    CK(f.lineBreakMode == NSLineBreakByWordWrapping && NSLineBreakByWordWrapping == 0);
    CK(!f.usesSingleLineMode && f.maximumNumberOfLines == 0);
    CK(f.alignment == NSTextAlignmentLeft && NSTextAlignmentLeft == 0);
    CK(f.acceptsFirstResponder && f.isFlipped);
    CKF(f.intrinsicContentSize.width, -1);
    value_f("sys_intrinsic_h", f.intrinsicContentSize.height);
    return finish();
}
