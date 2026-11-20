/* A timer source: first fire no earlier than the start time, then every
 * interval; the event handler's data counts the fires since it last ran.
 * Only lower bounds on time are asserted. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("com.poly.timer", DISPATCH_QUEUE_SERIAL);
    dispatch_source_t t = dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER, 0, 0, q);
    dispatch_semaphore_t got = dispatch_semaphore_create(0);
    long long t0 = gcd_now_ns();
    __block long long first_at = 0, fifth_at = 0;
    __block unsigned long total = 0;
    __block int calls = 0, on_queue = 1;
    dispatch_source_set_timer(t, dispatch_time(DISPATCH_TIME_NOW, 100 * NSEC_PER_MSEC), 30 * NSEC_PER_MSEC, 0);
    dispatch_source_set_event_handler(t, ^{
        if (strcmp(dispatch_queue_get_label(DISPATCH_CURRENT_QUEUE_LABEL), "com.poly.timer") != 0) on_queue = 0;
        total += dispatch_source_get_data(t);
        calls++;
        if (calls == 1) first_at = gcd_now_ns();
        if (calls == 5) {
            fifth_at = gcd_now_ns();
            dispatch_source_cancel(t);
        }
        dispatch_semaphore_signal(got);
    });
    dispatch_resume(t);
    gcd_wait(got);
    check("the first fire is not before the start time (100 ms)", first_at - t0 >= 100 * 1000000LL, (first_at - t0) / 1000000);
    for (int i = 0; i < 4; i++) gcd_wait(got);
    check("five handler calls", calls == 5, calls);
    check("the fifth is not before start + 4 intervals (100 + 120 ms)", fifth_at - t0 >= 220 * 1000000LL, (fifth_at - t0) / 1000000);
    check("the handler runs on the source's queue", on_queue, 0);
    check("the data over all calls counts at least the calls (never fewer)", total >= 5, (long long)total);
    gcd_pause_ms(150);
    check("after the cancel in the handler no more calls", calls == 5, calls);
    return finish();
}
