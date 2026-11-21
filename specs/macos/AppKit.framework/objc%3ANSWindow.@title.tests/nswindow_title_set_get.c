/* poly-mac: appkit */
/* title: a new window's title is the empty string (not nil); setTitle: stores a copy (a mutable string changed
 * later does not change the title), the getter returns an equal string; a title with non-ASCII
 * characters and with a very long text is kept whole; setting nil raises an exception (the title stays), the empty string gives an empty title;
 * the title can be set on a borderless window too (it only has no title bar to show it) and does not
 * change the frame. */
#include "poly_mak.h"

int test_main(void) {
    NSWindow *w = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 200, 100));
    CK(w.title != nil && w.title.length == 0);
    w.title = @"Hello";
    CK([w.title isEqualToString:@"Hello"]);
    NSMutableString *m = [NSMutableString stringWithString:@"abc"];
    w.title = m;
    [m appendString:@"def"];
    CK([w.title isEqualToString:@"abc"]);
    w.title = @"Grüße 中\U0001F600";
    CKV(w.title.length == 9, w.title.length);
    CK([w.title isEqualToString:@"Grüße 中\U0001F600"]);
    NSString *longt = [@"" stringByPaddingToLength:5000 withString:@"x" startingAtIndex:0];
    w.title = longt;
    CK(w.title.length == 5000);
    NSRect fr = w.frame;
    NSString *none = nil;
    NSString *exc = @"none";
    @try {
        w.title = none;
    } @catch (NSException *e) {
        exc = e.name;
    }
    value_ns("setTitle: nil raises", exc);
    CK(w.title.length == 5000);
    w.title = @"";
    CK(w.title != nil && w.title.length == 0);
    CK(NSEqualRects(fr, w.frame));
    NSWindow *b = ak_window(NSWindowStyleMaskBorderless, NSMakeRect(0, 0, 50, 50));
    b.title = @"x";
    CK([b.title isEqualToString:@"x"]);
    CKF(b.frame.size.height, 50);
    return finish();
}
