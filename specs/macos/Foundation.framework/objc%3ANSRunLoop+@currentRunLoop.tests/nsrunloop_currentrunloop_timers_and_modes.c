/* poly-mac: foundation */
/* A one-shot timer fires once during runUntilDate: (the run lasts to its date), a
 * repeating one about once per interval; invalidate stops it; inside the
 * callback currentMode is the running mode; performSelector:afterDelay: runs; a
 * timer added to a custom mode fires only when that mode runs. */
#include "poly_mfn.h"

int test_main(void) {
    NSRunLoop *rl = [NSRunLoop currentRunLoop];
    __block int fired = 0;
    __block NSString *mode = nil;
    [NSTimer scheduledTimerWithTimeInterval:0.05 repeats:NO block:^(NSTimer *t) {
        (void)t;
        fired++;
        mode = [[NSRunLoop currentRunLoop] currentMode];
    }];
    NSDate *t0 = [NSDate date];
    [rl runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.3]];
    CK(fired == 1 && [[NSDate date] timeIntervalSinceDate:t0] >= 0.25);
    CK(ns_is(mode, "kCFRunLoopDefaultMode"));
    __block int ticks = 0;
    NSTimer *rep = [NSTimer scheduledTimerWithTimeInterval:0.05 repeats:YES block:^(NSTimer *t) { (void)t; ticks++; }];
    [rl runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.5]];
    CK(ticks >= 5 && ticks <= 12);
    [rep invalidate];
    CK(!rep.isValid);
    int before = ticks;
    [rl runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.2]];
    CK(ticks == before);
    __block int custom = 0;
    NSTimer *ct = [NSTimer timerWithTimeInterval:0.02 repeats:NO block:^(NSTimer *t) { (void)t; custom++; }];
    [rl addTimer:ct forMode:@"PolyCustomMode"];
    [rl runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.2]];
    CK(custom == 0);
    [rl runMode:@"PolyCustomMode" beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.5]];
    CK(custom == 1);
    return finish();
}
