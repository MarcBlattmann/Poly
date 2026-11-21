/* poly-mac: appkit */
/* After the first draws (windows of this program are layer-backed): -display of the root draws the root only,
 * with its bounds; -display of a subview draws that subview with its own bounds (0,0,30,30);
 * marking a subview (needsDisplay) draws that subview alone (not the root) on the next turn, with
 * the root's bounds in the subview's coordinates (-20,-20,100,60); setNeedsDisplayInRect: on a subview draws
 * just that rectangle (1,1,5,5); setNeedsDisplayInRect: on the root over a subview draws only the root, with
 * that rectangle; a hidden subview marked as needing display is not drawn and is drawn again
 * (printed) after it is shown again. A window ordered out still draws a view marked as needing display (printed). */
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
static NSString *take(void) {
    NSString *s = [g_log copy];
    [g_log setString:@""];
    return s;
}

int test_main(void) {
    g_log = [NSMutableString string];
    DV *root = mk(@"root", NSMakeRect(0, 0, 100, 60));
    DV *sub = mk(@"sub", NSMakeRect(20, 20, 30, 30));
    DV *sub2 = mk(@"sub2", NSMakeRect(60, 5, 30, 30));
    [root addSubview:sub];
    [root addSubview:sub2];
    NSWindow *w = ak_window(NSWindowStyleMaskBorderless, NSMakeRect(0, 0, 100, 60));
    w.contentView = root;
    [w orderFront:nil];
    ak_spin(0.3);
    (void)take();
    ak_spin(0.2);
    CK([take() length] == 0);
    [root display];
    CK([take() isEqualToString:@"root(0,0,100,60);"]);
    [sub display];
    CK([take() isEqualToString:@"sub(0,0,30,30);"]);
    sub.needsDisplay = YES;
    ak_spin(0.2);
    CK([take() isEqualToString:@"sub(-20,-20,100,60);"]);
    [sub setNeedsDisplayInRect:NSMakeRect(1, 1, 5, 5)];
    ak_spin(0.2);
    CK([take() isEqualToString:@"sub(1,1,5,5);"]);
    [root setNeedsDisplayInRect:NSMakeRect(25, 25, 10, 10)];
    ak_spin(0.2);
    CK([take() isEqualToString:@"root(25,25,10,10);"]);
    sub.hidden = YES;
    sub.needsDisplay = YES;
    ak_spin(0.2);
    CK([take() length] == 0);
    sub.hidden = NO;
    ak_spin(0.2);
    value_ns("after unhide", take());
    [w orderOut:nil];
    root.needsDisplay = YES;
    ak_spin(0.2);
    value_ns("ordered out and marked", take());
    [w close];
    return finish();
}
