/* dispatch_source_merge_data from any thread; the handler sees the sum;
 * the data is reset for the next call; coalescing over many merges. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("com.poly.merge", DISPATCH_QUEUE_SERIAL);
    dispatch_source_t s = dispatch_source_create(DISPATCH_SOURCE_TYPE_DATA_ADD, 0, 0, q);
    dispatch_semaphore_t got = dispatch_semaphore_create(0);
    __block unsigned long total = 0;
    __block int calls = 0, zero_calls = 0;
    dispatch_source_set_event_handler(s, ^{
        unsigned long d = dispatch_source_get_data(s);
        if (d == 0) zero_calls++;
        total += d;
        calls++;
        dispatch_semaphore_signal(got);
    });
    dispatch_resume(s);
    dispatch_source_merge_data(s, 3);
    gcd_wait(got);
    check("a single merge of 3: data 3", total == 3, (long long)total);

    dispatch_queue_t g = dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0);
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    for (int t = 0; t < 8; t++)
        dispatch_async(g, ^{
            for (int i = 0; i < 1000; i++) dispatch_source_merge_data(s, 1);
            dispatch_semaphore_signal(done);
        });
    for (int t = 0; t < 8; t++) gcd_wait(done);
    gcd_pause_ms(200);
    check("8 threads x 1000 merges of 1: the handler's data adds up to 8003 in all", total == 8003, (long long)total);
    check("in fewer handler calls than merges (they coalesce)", calls < 8001, calls);
    check("the handler never saw a zero", zero_calls == 0, zero_calls);
    dispatch_source_cancel(s);
    return finish();
}
