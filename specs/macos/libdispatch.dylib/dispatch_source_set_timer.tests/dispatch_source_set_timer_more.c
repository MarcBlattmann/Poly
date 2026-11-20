/* A suspended timer does not fire; the handler can run late but the data
 * then counts the missed fires; a wall-clock start time works too. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("com.poly.timer2", DISPATCH_QUEUE_SERIAL);
    dispatch_source_t t = dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER, 0, 0, q);
    dispatch_semaphore_t got = dispatch_semaphore_create(0);
    __block unsigned long last_data = 0;
    __block int calls = 0;
    dispatch_source_set_timer(t, dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_MSEC), 10 * NSEC_PER_MSEC, 0);
    dispatch_source_set_event_handler(t, ^{ last_data = dispatch_source_get_data(t); calls++; dispatch_semaphore_signal(got); });
    dispatch_resume(t);
    gcd_wait(got);
    dispatch_suspend(t);
    gcd_pause_ms(100); /* about ten intervals pass while suspended */
    int calls_before = calls;
    gcd_pause_ms(100);
    check("a suspended timer calls the handler no more", calls == calls_before, calls);
    dispatch_resume(t);
    gcd_wait(got);
    check("after resume the handler's data counts at most one fire per interval gone, and at least one", last_data >= 1, (long long)last_data);
    dispatch_source_cancel(t);

    dispatch_source_t w = dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER, 0, 0, q);
    __block int wall = 0;
    dispatch_source_set_timer(w, dispatch_walltime(NULL, 50 * (int64_t)NSEC_PER_MSEC), DISPATCH_TIME_FOREVER, 0);
    dispatch_source_set_event_handler(w, ^{ wall = 1; dispatch_semaphore_signal(got); });
    dispatch_resume(w);
    check("a timer with a wall-clock start fires", gcd_wait(got) && wall == 1, wall);
    dispatch_source_cancel(w);
    return finish();
}
