/* DATA_OR: flags merged from several threads are ORed; replaced by the
 * handler's read of 0 for the next round. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("com.poly.merge-or", DISPATCH_QUEUE_SERIAL);
    dispatch_source_t s = dispatch_source_create(DISPATCH_SOURCE_TYPE_DATA_OR, 0, 0, q);
    dispatch_semaphore_t got = dispatch_semaphore_create(0);
    __block unsigned long seen_flags = 0;
    dispatch_source_set_event_handler(s, ^{ seen_flags |= dispatch_source_get_data(s); dispatch_semaphore_signal(got); });
    dispatch_resume(s);
    for (int bit = 0; bit < 8; bit++) {
        dispatch_async(dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^{ dispatch_source_merge_data(s, 1UL << bit); });
    }
    gcd_pause_ms(200);
    while (dispatch_semaphore_wait(got, DISPATCH_TIME_NOW) == 0) {}
    check("eight flags merged from eight tasks all arrived (0xff)", seen_flags == 0xff, (long long)seen_flags);
    seen_flags = 0;
    dispatch_source_merge_data(s, 0x10);
    gcd_wait(got);
    check("a later merge starts from 0: only the new flag", seen_flags == 0x10, (long long)seen_flags);
    dispatch_source_cancel(s);
    return finish();
}
