/* poly-mac: appkit */
/* stop: alone does not end run at once: it takes effect when the next event is handled. A timer
 * at 0.1 s sends stop:, another at 0.6 s posts an event: run returns after the event, not at
 * the stop (at least 0.5 s), while isRunning already reads NO right after stop: (the loop goes
 * on until the event). */
#include "poly_mak.h"

static int running_after_stop = -1;

int test_main(void) {
    NSApplication *a = ak_init();
    NSTimer *t1 = [NSTimer timerWithTimeInterval:0.1 repeats:NO block:^(NSTimer *x) {
        (void)x;
        [NSApp stop:nil];
        running_after_stop = NSApp.isRunning;
    }];
    NSTimer *t2 = [NSTimer timerWithTimeInterval:0.6 repeats:NO block:^(NSTimer *x) {
        (void)x;
        [NSApp postEvent:ak_event(0, 0) atStart:YES];
    }];
    [[NSRunLoop currentRunLoop] addTimer:t1 forMode:NSRunLoopCommonModes];
    [[NSRunLoop currentRunLoop] addTimer:t2 forMode:NSRunLoopCommonModes];
    NSDate *t0 = [NSDate date];
    [a run];
    NSTimeInterval d = [[NSDate date] timeIntervalSinceDate:t0];
    CK(d >= 0.55 && d < 3.0);
    CK(running_after_stop == 0);
    CK(!a.isRunning);
    return finish();
}
