/* poly-mac: appkit */
/* Which views are drawn when a window shows a view tree: the first draws after ordering front call
 * drawRect: of the root view first, then of the subviews: each visible subview exactly once, its own
 * subviews right after it, a hidden subview not at all. The windows of this program are layer-backed,
 * and then every view is drawn on its own: the order of the sibling subviews is the REVERSE of the
 * `subviews` array here (the last added before the first; printed), and the dirty rectangle each
 * subview gets is not its own bounds but the root's bounds in the subview's own coordinates
 * (a: (-10,-10,100,100) for a subview at (10,10) of a 100 x 100 root; a1 at (5,5) in it: (-15,-15,100,100)); a
 * subview outside the root's bounds (500,500) is drawn as well. */
#include "poly_mak.h"

static NSMutableString *g_log;
@interface DV : NSView
@property(nonatomic, copy) NSString *name;
@end
@implementation DV
- (void)drawRect:(NSRect)r {
    [g_log appendFormat:@"%@(%g,%g,%g,%g);", self.name, r.origin.x, r.origin.y, r.size.width, r.size.height];
}
@end
static DV *mk(NSString *n, NSRect f) {
    DV *v = [[DV alloc] initWithFrame:f];
    v.name = n;
    return v;
}
static int count(NSString *s) {
    int n = 0;
    NSRange r = NSMakeRange(0, g_log.length);
    while (r.length) {
        NSRange f = [g_log rangeOfString:s options:0 range:r];
        if (f.location == NSNotFound) break;
        n++;
        r = NSMakeRange(f.location + f.length, g_log.length - f.location - f.length);
    }
    return n;
}

int test_main(void) {
    g_log = [NSMutableString string];
    DV *root = mk(@"root", NSMakeRect(0, 0, 100, 100));
    DV *a = mk(@"a", NSMakeRect(10, 10, 30, 30));
    DV *b = mk(@"b", NSMakeRect(50, 10, 30, 30));
    DV *a1 = mk(@"a1", NSMakeRect(5, 5, 10, 10));
    DV *h = mk(@"h", NSMakeRect(10, 50, 10, 10));
    h.hidden = YES;
    [root addSubview:a];
    [root addSubview:b];
    [a addSubview:a1];
    [root addSubview:h];
    NSWindow *w = ak_window(NSWindowStyleMaskBorderless, NSMakeRect(0, 0, 100, 100));
    w.contentView = root;
    [w orderFront:nil];
    ak_spin(0.3);
    value_ns("log", g_log);
    CK([g_log hasPrefix:@"root(0,0,100,100);"]);
    CK(count(@"a(") == 1 && count(@"b(") == 1 && count(@"a1(") == 1 && count(@"root(") == 1);
    CK(count(@"h(") == 0);
    CK([g_log rangeOfString:@"a(-10,-10,100,100);"].location != NSNotFound);
    CK([g_log rangeOfString:@"a1(-15,-15,100,100);"].location != NSNotFound);
    CK([g_log rangeOfString:@"b(-50,-10,100,100);"].location != NSNotFound);
    NSUInteger ia = [g_log rangeOfString:@"a("].location, ib = [g_log rangeOfString:@"b("].location, i1 = [g_log rangeOfString:@"a1("].location;
    CK(i1 > ia);
    CK(ib < ia);
    [w close];
    return finish();
}
