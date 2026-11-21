/* poly-mac: appkit */
/* While run is running the main run loop works: a one-shot timer set before run fires once, a
 * repeating one about once per interval (0.05 s over 0.5 s: 5 to 12 times); run returns after
 * the timer that stops it, and the run lasted about as long as that timer's delay. */
#include "poly_mak.h"

int test_main(void) {
    NSApplication *a = ak_init();
    __block int once = 0, rep = 0;
    [NSTimer scheduledTimerWithTimeInterval:0.05 repeats:NO block:^(NSTimer *t) { (void)t; once++; }];
    [NSTimer scheduledTimerWithTimeInterval:0.05 repeats:YES block:^(NSTimer *t) { (void)t; rep++; }];
    ak_stop_run_after(0.5);
    NSDate *t0 = [NSDate date];
    [a run];
    NSTimeInterval d = [[NSDate date] timeIntervalSinceDate:t0];
    CK(d >= 0.45 && d < 3.0);
    CK(once == 1);
    CK(rep >= 5 && rep <= 12);
    CK(!a.isRunning);
    return finish();
}
