/* poly-mac: appkit */
/* initWithFrame: never resizes the button (a 10 x 5 frame stays 10 x 5, a 500 x 300 frame stays); setTitle:
 * changes the title and not the frame; the title is copied (a mutable string changed later does not change
 * it), may be empty or contain any characters, and attributedTitle shows the same text; setting the title
 * to an empty string keeps the button usable; setting a title makes alternateTitle independent; the button is
 * a subclass of NSControl and NSView; its cell is not used for drawing in the program (cell may be nil or an
 * NSButtonCell; only the class check is made). */
#include "poly_mak.h"

int test_main(void) {
    NSButton *s = [[NSButton alloc] initWithFrame:NSMakeRect(0, 0, 10, 5)];
    NSButton *l = [[NSButton alloc] initWithFrame:NSMakeRect(0, 0, 500, 300)];
    CK(NSEqualSizes(s.frame.size, NSMakeSize(10, 5)) && NSEqualSizes(l.frame.size, NSMakeSize(500, 300)));
    CK([s isKindOfClass:[NSControl class]] && [s isKindOfClass:[NSView class]]);
    NSRect fr = s.frame;
    s.title = @"A longer title than the frame";
    CK(NSEqualRects(fr, s.frame));
    NSMutableString *m = [NSMutableString stringWithString:@"abc"];
    s.title = m;
    [m appendString:@"def"];
    CK([s.title isEqualToString:@"abc"]);
    CK([s.attributedTitle.string isEqualToString:@"abc"]);
    s.title = @"Grüße 中";
    CK([s.title isEqualToString:@"Grüße 中"]);
    s.title = @"";
    CK(s.title != nil && s.title.length == 0);
    s.alternateTitle = @"Alt";
    s.title = @"T";
    CK([s.alternateTitle isEqualToString:@"Alt"]);
    CK(s.cell == nil || [s.cell isKindOfClass:[NSButtonCell class]]);
    return finish();
}
