/* poly-mac: appkit */
/* setNeedsDisplay:YES does not draw at once: right after the call drawRect: has not run; the next run-loop
 * turn draws the whole bounds once; calling it several times before the turn still draws once; a view that
 * has just been drawn is not marked; setNeedsDisplay:NO after YES does not cancel the pending draw
 * (observed: the view still draws on the next turn). */
#include "poly_mak.h"

@interface DV : NSView
@property(nonatomic, strong) NSMutableString *log;
@end
@implementation DV
- (void)drawRect:(NSRect)r {
    [self.log appendFormat:@"draw(%g,%g,%g,%g);", r.origin.x, r.origin.y, r.size.width, r.size.height];
}
@end

int test_main(void) {
    DV *d = [[DV alloc] initWithFrame:NSMakeRect(0, 0, 100, 60)];
    d.log = [NSMutableString string];
    NSWindow *w = ak_window(NSWindowStyleMaskBorderless, NSMakeRect(0, 0, 100, 60));
    w.contentView = d;
    [w orderFront:nil];
    ak_spin(0.3);
    [d.log setString:@""];
    d.needsDisplay = YES;
    CK(d.log.length == 0);
    d.needsDisplay = YES;
    d.needsDisplay = YES;
    ak_spin(0.2);
    CK([d.log isEqualToString:@"draw(0,0,100,60);"]);
    value("needsDisplay after draw", d.needsDisplay);
    [d.log setString:@""];
    d.needsDisplay = YES;
    d.needsDisplay = NO;
    ak_spin(0.2);
    value_ns("YES then NO", d.log);
    [w close];
    return finish();
}
