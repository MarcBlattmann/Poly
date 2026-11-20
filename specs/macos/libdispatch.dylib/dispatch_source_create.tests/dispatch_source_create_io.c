/* READ and WRITE sources on a pipe: the read source fires when data are
 * waiting and its data is the number of bytes available; the write source
 * fires when the pipe has room. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"
#include <fcntl.h>

int test_main(void) {
    static int fds[2];
    if (pipe(fds) != 0) return 1;
    fcntl(fds[0], F_SETFL, O_NONBLOCK);
    fcntl(fds[1], F_SETFL, O_NONBLOCK);
    dispatch_queue_t q = dispatch_queue_create("com.poly.io", DISPATCH_QUEUE_SERIAL);
    dispatch_semaphore_t got = dispatch_semaphore_create(0);

    dispatch_source_t r = dispatch_source_create(DISPATCH_SOURCE_TYPE_READ, fds[0], 0, q);
    check("a read source is made", r != NULL, 0);
    check("get_handle is the descriptor", dispatch_source_get_handle(r) == (uintptr_t)fds[0], 0);
    __block unsigned long avail = 0;
    __block int fired = 0;
    dispatch_source_set_event_handler(r, ^{
        avail = dispatch_source_get_data(r);
        char buf[64];
        read(fds[0], buf, sizeof buf);
        fired++;
        dispatch_semaphore_signal(got);
    });
    dispatch_resume(r);
    check("nothing to read: the read source did not fire", dispatch_semaphore_wait(got, dispatch_time(DISPATCH_TIME_NOW, 150 * NSEC_PER_MSEC)) != 0, fired);
    write(fds[1], "hello", 5);
    check("after a write: it fired", gcd_wait(got), 0);
    check("with the number of bytes available as data (5)", avail == 5, (long long)avail);

    dispatch_source_t w = dispatch_source_create(DISPATCH_SOURCE_TYPE_WRITE, fds[1], 0, q);
    __block int wfired = 0;
    dispatch_source_set_event_handler(w, ^{ wfired = 1; dispatch_source_cancel(w); dispatch_semaphore_signal(got); });
    dispatch_resume(w);
    check("a write source on a pipe with room fires", gcd_wait(got) && wfired == 1, wfired);
    dispatch_source_cancel(r);
    return finish();
}
