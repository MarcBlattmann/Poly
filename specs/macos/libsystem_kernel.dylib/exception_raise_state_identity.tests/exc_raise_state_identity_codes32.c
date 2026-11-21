/* poly-mac: libsystem
 * The messages of a task exception port with EXCEPTION_STATE_IDENTITY (id 2403): the thread and task port descriptors and the thread state; the reply carries the new state. */
#include "poly_mexc.h"
int test_main(void) {
    exc_handler_t h;
    exc_handler_start(&h);
    h.set_reg = 1;
    h.reg_val = 0x1234567890abcdefUL;
    exc_seen_t s[3];
    kern_return_t kr = exc_three_faults(&h, EXCEPTION_STATE_IDENTITY, s);
    check("task_set_exception_ports succeeds", kr == KERN_SUCCESS, kr);
    exc_check_layout(2403, 0, s, &h);
    check("each fault was resumed: the thread went on", h.count == 3, h.count);
    check("the register the handler set is what the thread stored", exc_reg_out == 0x1234567890abcdefUL, (long long)exc_reg_out);
    return finish();
}
