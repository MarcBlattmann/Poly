/* poly-mac: libsystem
 * The same with MACH_EXCEPTION_CODES in the behavior: id 2405, 84 bytes, the codes are 64 bits wide and the address of the access is whole. */
#include "poly_mexc.h"
int test_main(void) {
    exc_handler_t h;
    exc_handler_start(&h);
    h.set_reg = 1;
    h.reg_val = 0x1234567890abcdefUL;
    exc_seen_t s[3];
    kern_return_t kr = exc_three_faults(&h, EXCEPTION_DEFAULT|MACH_EXCEPTION_CODES, s);
    check("task_set_exception_ports succeeds", kr == KERN_SUCCESS, kr);
    exc_check_layout(2401, 1, s, &h);
    check("each fault was resumed: the thread went on", h.count == 3, h.count);
    check("the register the handler set is what the thread stored", exc_reg_out == 0x1234567890abcdefUL, (long long)exc_reg_out);
    return finish();
}
