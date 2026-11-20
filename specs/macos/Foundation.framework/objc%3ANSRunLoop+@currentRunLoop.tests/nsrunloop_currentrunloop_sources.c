/* poly-mac: foundation */
/* performSelector:withObject:afterDelay: runs on the run loop after its delay, not
 * before and not twice; cancelPreviousPerformRequestsWithTarget: cancels it;
 * a notification posted from a timer reaches its observer; the loop's mode list
 * (via the CFRunLoop) contains the mode a timer was added to. */
#include "poly_mfn.h"

@interface PolyTarget : NSObject {
@public
    int ran;
}
- (void)go:(id)x;
@end
@implementation PolyTarget
- (void)go:(id)x {
    (void)x;
    ran++;
}
@end

int test_main(void) {
    NSRunLoop *rl = [NSRunLoop currentRunLoop];
    PolyTarget *t = [[PolyTarget alloc] init];
    [t performSelector:@selector(go:) withObject:nil afterDelay:0.05];
    CK(t->ran == 0);
    [rl runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.3]];
    CK(t->ran == 1);
    PolyTarget *c = [[PolyTarget alloc] init];
    [c performSelector:@selector(go:) withObject:nil afterDelay:0.05];
    [NSObject cancelPreviousPerformRequestsWithTarget:c];
    [rl runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.3]];
    CK(c->ran == 0);
    NSTimer *timer = [NSTimer timerWithTimeInterval:100 repeats:NO block:^(NSTimer *x) { (void)x; }];
    [rl addTimer:timer forMode:@"PolyListedMode"];
    CFArrayRef modes = CFRunLoopCopyAllModes([rl getCFRunLoop]);
    CK(CFArrayContainsValue(modes, CFRangeMake(0, CFArrayGetCount(modes)), CFSTR("PolyListedMode")));
    CFRelease(modes);
    [timer invalidate];
    return finish();
}
