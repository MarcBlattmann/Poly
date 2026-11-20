/* The cancel handler of a read source is where the descriptor may be
 * closed: it runs when the source no longer uses it. A source cancelled
 * while suspended runs its cancel handler only after the resume. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"
#include <fcntl.h>

int test_main(void) {
    static int fds[2];
    if (pipe(fds) != 0) return 1;
    dispatch_queue_t q = dispatch_queue_create("com.poly.cancel-io", DISPATCH_QUEUE_SERIAL);
    dispatch_source_t r = dispatch_source_create(DISPATCH_SOURCE_TYPE_READ, fds[0], 0, q);
    dispatch_semaphore_t cancelled = dispatch_semaphore_create(0);
    __block int closed = 0;
    dispatch_source_set_event_handler(r, ^{ char b[8]; read(fds[0], b, sizeof b); });
    dispatch_source_set_cancel_handler(r, ^{ closed = close(fds[0]) == 0; dispatch_semaphore_signal(cancelled); });
    dispatch_resume(r);
    dispatch_source_cancel(r);
    check("the cancel handler ran", gcd_wait(cancelled), 0);
    check("and closed the descriptor", closed, 0);

    dispatch_source_t d = dispatch_source_create(DISPATCH_SOURCE_TYPE_DATA_ADD, 0, 0, q);
    __block int ran = 0;
    dispatch_source_set_event_handler(d, ^{});
    dispatch_source_set_cancel_handler(d, ^{ ran = 1; dispatch_semaphore_signal(cancelled); });
    dispatch_source_cancel(d); /* never resumed */
    gcd_pause_ms(150);
    check("a source cancelled before it was resumed has not run its cancel handler", ran == 0, ran);
    dispatch_resume(d);
    check("after the resume it does", gcd_wait(cancelled) && ran == 1, ran);
    close(fds[1]);
    return finish();
}
