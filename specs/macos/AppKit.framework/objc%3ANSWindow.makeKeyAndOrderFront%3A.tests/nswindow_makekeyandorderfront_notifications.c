/* poly-mac: appkit */
/* After makeKeyAndOrderFront: and a turn of the run loop the window is visible and the public
 * notifications NSWindowDidBecomeKeyNotification and NSWindowDidBecomeMainNotification have not
 * arrived (this program is not active, so the window is neither key nor main), nor has
 * NSWindowDidResignKeyNotification; the window delegate gets no windowDidBecomeKey:. Private
 * notification names and the number of layout notifications are not part of this entry. */
#include "poly_mak.h"

@interface D : NSObject <NSWindowDelegate>
@property(nonatomic) int key;
@end
@implementation D
- (void)windowDidBecomeKey:(NSNotification *)n { (void)n; self.key++; }
@end

int test_main(void) {
    NSMutableString *log = [NSMutableString string];
    NSWindow *w = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 200, 100));
    D *d = [D new];
    w.delegate = d;
    id tok = [[NSNotificationCenter defaultCenter] addObserverForName:nil object:w queue:nil usingBlock:^(NSNotification *x) {
        [log appendFormat:@"%@,", x.name];
    }];
    [w makeKeyAndOrderFront:nil];
    ak_spin(0.2);
    [[NSNotificationCenter defaultCenter] removeObserver:tok];
    CK(w.isVisible);
    CK([log rangeOfString:@"NSWindowDidBecomeKeyNotification"].location == NSNotFound);
    CK([log rangeOfString:@"NSWindowDidBecomeMainNotification"].location == NSNotFound);
    CK([log rangeOfString:@"NSWindowDidResignKeyNotification"].location == NSNotFound);
    CK(d.key == 0);
    [w close];
    return finish();
}
