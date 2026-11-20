/* poly-mac: libsystem
 * The same with MACH_EXCEPTION_CODES: id 2407, 64-bit codes. */
#include "poly_mexc.h"
int test_main(void) {
    exc_handler_t h;
    exc_handler_start(&h);
    h.set_reg = 1;
    h.reg_val = 0x1234567890abcdefUL;
    exc_seen_t s[3];
    kern_return_t kr = exc_three_faults(&h, EXCEPTION_STATE_IDENTITY|MACH_EXCEPTION_CODES, s);
    check("task_set_exception_ports succeeds", kr == KERN_SUCCESS, kr);
    exc_check_layout(2403, 1, s, &h);
    check("each fault was resumed: the thread went on", h.count == 3, h.count);
    check("the register the handler set is what the thread stored", exc_reg_out == 0x1234567890abcdefUL, (long long)exc_reg_out);
    return finish();
}
